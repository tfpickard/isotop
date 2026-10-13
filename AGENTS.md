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
   Two per-process sources go through `MemoryReads`: `Source::Oom` ranks every pid the cycle
   scanned (`oom_score`, `oom_score_adj`), and `Source::MemoryDetail` reads `smaps_rollup` for
   the `ROLLUP_PROCESSES` largest, which `Collector::sample` picks with `top_n` and hands over in
   `rollup_pids`; the pass stops after `ROLLUP_BUDGET` and resumes where it stopped, and the
   processes it did not reach keep their last reading (`Kept`). A kernel before 5.3 reports
   `Source::MemorySplit` missing.
   Optional sources, the ones only some views draw, are read only while the shown view declares
   them: `main.rs` hands `Collector::demand` the `sources` of the shown view's registry row at
   startup and on every view change, the thread takes a copy of that `Demand` at the start of
   each cycle, and reads an optional source through `Demand::read_if` (or `Reads::read`, which
   also records it as active or, when it cannot be read, missing). Remote locations
   (`Source::Locations`, the GeoIP lookups the globe draws) and pipes (`Source::Pipes`, the
   descriptor walk and fdinfo reads behind `Snapshot.pipes`) go through it; the unread bytes of
   Unix sockets (`Snapshot.unix_queues`) ride the socket scan, which every view reads, and are not
   demanded. The journal follower in `main.rs` runs while the demand contains `Source::Journal` and for
   `JOURNAL_LINGER` (30 s) after; no view is special-cased. While the demand contains it, `feed` attaches
   each frame's new lines (the follower's, or the demo's) to the newest sample as `Snapshot.journal`
   (`journal::Delivery`, numbered so a view takes each line once), and the view that draws them reads
   them from its snapshot. A sample that stops being the newest (`App::push_sample`) lets go of its
   lines and keeps their numbering, so the history holds one sample's lines (at most 2048), not
   `--history` times that. The kernel counters (`Source::Interrupts`, `Softirqs`,
   `IrqAffinity` and `OomKills`; `platform/linux/counters.rs`, `model/kernel.rs`) go through it
   too: with none demanded no `/proc` counter file is opened, and `Snapshot.interrupts`,
   `softirqs` and `oom_kills` stay Pending, Pending and None. Threads (`Source::Threads`) are read
   the same way: while they are demanded each sample hands the thread the processes busiest
   first (`threads::targets`, CPU descending, ties by `Identity`), and a pass
   (`threads::ThreadScan`) walks that list, reading at most `THREADS_PER_PASS` (8,192) threads
   and `THREADS_PER_PROCESS` (1,024) per process and never sorting on its own. A process cut
   short, or not reached, has a `thread_list` that is not `complete`. `Snapshot.sources` lists the
   optional sources read for the sample, and `Snapshot.missing` those demanded and unreadable;
   one nobody demanded is in neither. `--demo` starts no thread, so `sources` is empty there.
   While the demand contains `Source::LifeEvents`, `Collector::sample` also runs a second thread,
   `isotop-events` (`platform/linux/events.rs`: the proc connector and taskstats;
   `platform/macos/events.rs`: kqueue), which fills a bounded `LifeQueue` that each sample drains
   and merges with sample diffing into `Snapshot.life`; dropping the queue when the demand ends
   stops it. On Linux the sampler caches each process's cmdline and cgroup per `Identity` and re-reads them after an
   exec or every few samples, so the frame loop reads only `stat` and `io` per process.
   `main.rs` keeps a history ring of snapshots for pause and rewind.
2. **Record** (`render.rs`, `views.rs`). `Scene::render` turns a snapshot into a display list of
   screen-space `Item`s (triangles, field triangles, lines, sphere impostors, glows, beams, stars) on a `Frame`.
   It eases the sample, admits the processes, and hands the shown view's module (`ViewModule::draw`)
   a `Stage`; the scene then draws the links and exit flashes the view's registry row asks for.
   The `Scene` persists between frames only what is frame-wide: smoothing, births and deaths,
   links and highlights, the last frame's positions, places, bounds and notes, the shared history,
   spare buffers and the visible and collapsed counts. Each view's own state (layout, simulated
   media) lives in its module, which the scene builds from the registry row and owns.
3. **Rasterize**. `Frame::rasterize` does this on the CPU; `gpu.rs` with `shaders.wgsl` does it
   on wgpu (Vulkan, Metal or DX12). Both produce colour, depth and a pick buffer.
4. **Present** (`terminal.rs`). Frames go to the terminal through POSIX shared memory (`t=s`),
   or inline as zlib and base64. Labels, popups and the status lines are drawn as terminal
   text over the image.

| File | Owns |
|---|---|
| `main.rs` | CLI (`Options`), `Layout`, `App` (input, camera easing, tour and the Space visits that share its cursor over `visit_cycle`, overlay text, when the journal follower runs), `preflight` (`--verbose`, `--list-views` and views this platform lacks, before the terminal is touched), main loop, `demo_snapshot` (every demo sample goes through it with the shown view's `DemoOptions`, and the time of the sample before, for the life events in between), headless PNG and benchmark, and `warm_up`, which replays the shared history (headless demo runs, unless the row's `Phase::History` fills it) and runs the shown view's warm-up phases before a headless frame (the golden render test calls it too) |
| `app/mod.rs`, `app/camera.rs`, `app/fixtures.rs`, `app/input.rs`, `app/overlay.rs`, `app/pointer.rs`, `app/screen.rs`, `app/search.rs`, `app/status.rs`, `app/switcher.rs`, `app/tour.rs` | `App` split by concern (placeholder row, replaced as the components land) |
| `model.rs` | `Process`, `Snapshot`, `Collector` (CPU smoothing, per-interval rates and shares from counter deltas, background thread, merge), `Demand` and `Reads` (which optional sources the background thread reads), `top_n` (the budgeted selection: largest key first, ties by `Identity`, NaN last), `Life` (merges the event thread's observations with sample diffing into `Snapshot.life`: one exit per `Identity`, never revised), `Oom`, `Rollup` and `ROLLUP_PROCESSES` (the 64 largest processes get `Process::memory_detail`, read in a 50 ms budget that resumes where it stopped), `MemoryReads` (the two memory sources of the background thread) and `Kept` (readings kept between cycles per `Identity`), demo workload (`demo_with` with the `DemoOptions` a view's row asks for: `churn` for deterministic births and exits, which `demo_life` lists and `demo_events` turns into life events, `exact_cpu_time`, `observer` for `Snapshot::observer`, isotop itself, `mortality`, which shapes the churn's deaths for a graveyard (every cause, a burst, a stopped unit, a reused pid), `oom_and_memory_detail`, which gives every process an `Oom` (three System processes exempt) and the 64 largest a consistent `Rollup`, one of them unreadable, and `kernel_counters`, which synthesizes `Snapshot.interrupts`, `softirqs` and `oom_kills`, the last the sum of the demo units' `oom_kills`, and `threads`, which gives every process the threads `threads::demo_list` synthesizes; plain `demo` leaves all of these unread; `channels`, which synthesizes `Snapshot::unix_queues` and `Snapshot::pipes`; `DEMO_EPOCH`, the wall clock of demo time 0; `demo` is the unoptioned test shorthand), `Queue` and `Pipe` with `pair_pipes` (pairs the pipe and FIFO descriptors of every process by `PipeId` into reader and writer links), `describe` |
| `model/kernel.rs` | The kernel counters of the background thread: `KernelScan` (one pass: reads each demanded source through `Reads`, joins the affinity onto the interrupt rows, turns counts into per-CPU rates from the change since the previous pass, None on a first pass, a new row or CPU and a count that went backwards), `timer_row` (`LOC`, or the `arch_timer` row on arm64), `Interrupt::is_numbered`, and the demo's synthesized interrupts and softirqs (`demo`, pure functions of time) |
| `platform/mod.rs` | The OS contract (`Sampler`, `RawProcess`, `network`, `account`, `FileScan`, `locks`, `Gpu`, `open_events`, `journal`, the kernel counters `interrupts`, `softirqs`, `irq_affinity` and `oom_kills`, and `oom_scores`, `rollup` and `Taken`, one reading of a process that may be gone or unreadable), the `Source` registry and `Sources` bit set that `Snapshot::missing` holds, the platform-neutral `Network` (with the Unix socket queues), `Files`, `Locks` and the pipe types (`PipeScan`'s `PipeSample` of `PipeEnd`s, `PipeId`, `AccessMode`), and the life event types (`LifeEvent`, `Cause`, `ExitRecord`, what an event source `Observed`, and the `LifeQueue` it fills); selects the implementation by `target_os` |
| `platform/linux/mod.rs` | Linux `Sampler`: `/proc` parsing, CPUs, pressure, core kinds, cgroup v2 accounting, and the gaps it reports through `missing` (cpufreq, schedstat, core kinds, cgroup v2) |
| `platform/linux/net.rs` | Socket links: `/proc/net/tcp*`, sock_diag netlink (Unix peers with their receive queues from `UNIX_DIAG_RQLEN`, inet TCP with `tcp_info`) |
| `platform/linux/memory.rs` | The out-of-memory ranking (`oom_score`, `oom_score_adj`) and one process's `smaps_rollup` in bytes, with their parsers; kernels before 5.3 have no `Pss_` split, which is left unknown |
| `platform/linux/files.rs` | Open regular files and deleted-but-open files per process (`/proc/<pid>/fd`, bounded per scan) and file locks with their waiters (`/proc/locks`) |
| `platform/linux/fdinfo.rs` | `/proc/<pid>/fdinfo/<fd>`: `parse_fdinfo` (flags in octal, offset) and the access mode from `O_ACCMODE`, shared by the pipe scan and per-file attribution |
| `platform/linux/pipes.rs` | `PipeScan`, behind `Source::Pipes`: `pipe:[inode]` descriptors and FIFOs (a stat), direction from fdinfo, at most 4096 fdinfo reads per pass round-robin by pid |
| `platform/linux/nvml.rs` | NVIDIA per-process GPU memory via `dlopen`; never wakes a runtime-suspended GPU |
| `platform/linux/journal.rs` | `journalctl` in export format and its parser |
| `platform/linux/counters.rs` | `/proc/interrupts` (columns follow the `CPUn` header, numbered and named rows, totals for rows with fewer counts than CPUs), `/proc/softirqs`, `effective_affinity_list` per numbered IRQ, `parse_cpu_list`, and `oom_kill` from `/proc/vmstat`; fixtures in `platform/linux/fixtures/`, real captures where the header says so |
| `threads.rs` | Per-thread sampling (`Source::Threads`): `Thread`, `ThreadList` (in `Process::thread_list`), the caps, `ThreadReader` (what each platform's `Threads` provides), `DeltaTracker` (cumulative counters to growth, forgetting keys a pass missed) and `ThreadScan` (the capped pass and per-thread CPU keyed `(tid, start)`), `targets` (the order the frame thread hands over) and `demo_list` (the demo's threads, `DemoOptions::threads`) |
| `platform/linux/threads.rs` | The Linux `ThreadReader`: `/proc/<pid>/task/<tid>/stat`, parsed after the last `)` with the field indices `parse_stat` uses (`split_stat`, `timing` in `platform/linux/mod.rs`) |
| `platform/linux/events.rs` | The `isotop-events` thread on Linux: proc connector fork, exec and exit events and taskstats exit records over netlink, decoded by byte offset from the uapi headers, thread records summed per process |
| `journal.rs` | Journal lines for the Matrix view: reader thread, bounded backlog, demo lines, and `Delivery`, the lines one sample carries on `Snapshot.journal` |
| `views.rs` | The view registry: `REGISTRY`, one `ViewInfo` row per view in Tab order; `View`, an index into it; `Build`, the constructor a row holds for its module; the `ViewModule` trait, `ViewKey`, `Warmup`, `LinkRule` and the harness `Contract` (`Property`, `Unstable`); and `testing`, module views for tests |
| `views/city.rs` | The City module view (declared from `views.rs`): `City` keeps each process's plot (`Plot`: a district and one of its 16 slots, taken from the first vacant slot of its cgroup's districts and kept until the process leaves the sample), `district_origin` spirals the districts out in square shells without moving any, and a building is its height from CPU, footprint from memory, windows, rooftop lamp, GPU beacon and I/O pulse; it reports the tallest height in `Stage::headroom` for `Scene::fit`, and its row's `link_lift` of 1.5 raises the links over the rooftops |
| `views/systems.rs` | The process tree as nested Keplerian systems, which the Orbit and Flow module views share: `layout` (the trees laid out for one frame, as a `Layout` of `Path`s, `Body`s and `Star`s), `Tree`, `Walk`, `hubs` (a hollow `Process` stand-in for a parent isotop cannot read, with `HUB_START` identities that never equal a process's, so it is never pickable), `Galaxy` (each of Orbit and Flow owns one: the layout code is shared, the state is not, so switching from one to the other never shows wells at the places Orbit has turned its systems to; `place` reserves each system's room, with `HEADROOM` to spare for changes of memory and population, and keeps its place until the structure outgrows it, `revolve` turns the systems about their barycenter in Kepler bands, only across consecutive `Stage::frames`), `shells`, `elements`, `period`, `Orbit`, `system_label` and `hub_note`; `Star::at` is where a system's star stands (a hub is no process, so `Stage::positions` has none for it and the camera fit takes its place from its `Title`), and `Layout::report` tells the app the tour stops (`Stage::tour`, a hub left out), the system titles (`Stage::titles`) and the memory a system holds (`Stage::popup`), and `Layout::homes` (test only) gives each process its system's centre, the place the two views keep, for their `ViewModule::anchors` |
| `views/orbit.rs` | The Orbit module view (declared from `views.rs`): `Orbits` keeps a `Galaxy`, lays the systems out with the orbital phase following the scene's time, and draws the orbits, CPU glows and trails, the bodies (a hub as an outline), GPU halos and thread rings; its legend adds `gpu_note` and `systems::hub_note`, and its anchors are each process's system centre, since the orbits and slots inside follow memory and the other bodies; where a body stands inside its system is not yet stable under changes of memory and population, tracked in [#142](https://github.com/tfpickard/isotop/issues/142) |
| `views/flow.rs` | The Flow module view (declared from `views.rs`): `Wells` keeps a `Galaxy` and a `medium::Flow` sheet, lays the systems out with the bodies held still, and pours the weather into the sheet (`weather`: a bowl per system and a dimple per heavy process for memory, whirlpools for CPU, two-lane `Lane` rivers for sockets, sparks for connections leaving the machine), kicks the particles from `Stage::splashes` and `Stage::newborn`, and draws the fabric, the bodies sunk into it and the particles; its anchors are each process's system centre, the well it sinks into; where a body stands inside its system is not yet stable under changes of memory and population, tracked in [#142](https://github.com/tfpickard/isotop/issues/142) |
| `views/ripple.rs` | The Ripple module view (declared from `views.rs`): `Ripple` keeps the `medium::Wave` and the pebble `Cluster` of each cgroup (a sunflower spiral, a slot kept while its process lives, drawn or not; pebbles grow in place into the clearance left round them, so a cluster is spread out only when its largest would touch a neighbour, which no doubling of memory brings about), drives the wave from each busy process's own pitch and from the scene's births and exits (`Stage::newborn`, `Stage::splashes`), lights the water as facets and gives each patch to its nearest process (`territories`, which allocates its owner list and queue every frame); a cluster of two or more drawn pebbles is a system, named in `Stage::titles`, `Stage::tour` and `Stage::popup`. It draws with `render::cover` (the grid over the layout), `similar` and `glows`, which Flow shares |
| `harness.rs` | Test only: the shared view harness. `Session` runs a view headless through `warm_up` and `App::render`; `rescaled` and `churned` perturb a demo sample; `check` tests every `Property` of a row against its `Contract`, and `every_view_satisfies_the_view_contract` runs it for every registered view |
| `render.rs` | `Camera`, `Sky`, `Item`, `Frame` (CPU rasterizer and picking), `Thing`, `Target` and `Card` (non-process pickables and their inspector cards), `RenderInput`, `Stage` with its `dt` policy and `TourTarget`, and `Scene` (frame-wide state only, plus the module of each registry row, which it calls through `ViewModule`), and the shared ground-grid helpers `cover`, `footprint`, `similar` and `glows` that Ripple and Flow draw with |
| `render/shared.rs` | Constants the CPU rasterizer and `shaders.wgsl` must agree on, declared once; `prelude` writes them into the WGSL source. `LIGHT`, the one world light direction every lit surface shares, is Rust only: shading is baked into colours before the backends see them |
| `render/strokes.rs` | Wide strokes built only from `Item::Triangle`s: `Frame::ribbon` (a screen-facing band along a polyline, `Width` in world units or pixels, an optional `Dash` measured along the 3D arc length) and `Frame::tube` (a flat-shaded prism with parallel-transported `ring_frames`, lit by `shared::LIGHT` through `lit`; `tube_faces` is its pure world-space geometry); every piece is at most 32 px an edge, so a triangle's box is at most 64 x 64 px |
| `render/field.rs` | The field primitive (`Field`, `Placement`, `Relief`, `Shading`, `Frame::field`, `Item::Field`): a grid of colours drawn in world space, flat or as lit relief, sampled bilinearly by the one formula both backends use (clamped uv in node units, `floor(bilerp + 0.5)` per channel, the owner of the nearest node as the pick), with `ShadedField` (the shaded colours and owners a frame keeps and recycles through `Buffers`, so an unchanged `(key, revision)` is not shaded again), the CPU span rasterizer `Frame::field_triangle`, and `StorageLayout`, which plans what the GPU storage buffer must rewrite. Grid simulations draw through it, never one triangle pair per cell; Ripple and Flow still do |
| `arbor.rs` | The Arbor module view: the process tree as a tree. The trunk is PID 1 (else the root of the largest subtree, or a pale launchd), each child sits on a phyllotaxis slot (`Slots`) its parent hands out in the order it first sees it from a counter that only rises, and branch radius is `R_TRUNK * sqrt(subtree area / trunk area)` over stretches whose areas `Tree::levels` takes junction by junction (Leonardo's rule). Kernel processes below the trunk grow as roots under a soil slab, which is opaque, so `surface_distance` moves their points toward the viewer until they lie just in front of it. Leaves are threads (at most 32 per twig and 6,000 in all, `leaf_specs`), a `Fall` is the leaf of an exited thread, blossoms follow `Stage::age`, and each branch is a damped `Oscillator` stepped by `FixedStep` in a wind set by CPU pressure |
| `cores.rs`, `cells.rs`, `strata.rs`, `globe.rs`, `reef.rs` | Module views, each a state struct that implements `ViewModule` (`draw` and `legend`): the CPU core track, cgroup petri dishes, a minute of CPU history as ridges (Strata keeps only its ridge rows; the profile is read from the shared `History`), TCP connections on a turning earth, and the coral reef. Globe's `face`, `turn` and GeoIP `sources` stay on its row |
| `matrix.rs`, `glyphs.rs` | The Matrix view (`Rain`, drawn from journal lines, not processes) and its 7x14 bitmap font |
| `geo.rs`, `places.rs`, `coastline.bin` | Local GeoIP lookup and home from the time zone (`geo.rs`), country label points and legacy GeoIP country order (`places.rs`), and the packed coastlines the globe draws |
| `ptolemy.rs` | The Ptolemy module view: isotop as Earth, the seven most varied processes as planets on deferents carrying `simulation::dft` epicycle chains that replay their last minute, trails tinted where retrograde, and every other process a fixed star (`stellar`) with novae and fades from `History::changes` |
| `tesseract.rs` | The Tesseract module view: the `Worldlines` recorder (cumulative CPU time at most every 0.25 s for 180 s, recorded in every view) and its seat bands, the 4D rotation, the Lorentz boost and the display compression, then the Schlegel-projected worldlines, heads, fork vertices and I/O marks |
| `necropolis.rs` | The Necropolis module view: deaths from `Snapshot::life` buried by `record` in every view (rows by UTC minute, families together, mass graves, OOM claims, mausoleums for stopped units, unmarked markers, ghosts on pid reuse, the 600-grave cap), stones built once per grave, and the fogged cemetery, zombies in pits and the real moon drawn from them; `moon_age`, `moon_lit`, `lifetime_text` and `Death` |
| `neurons.rs` | The simulation behind Cortex, pure: Izhikevich neurons stepped by 0.5 ms ticks (`Firing`, `advance`, `calibration` and `current` from CPU), a `Network` of neurons, synapses and one event queue of spikes in flight, Golomb's synchrony from exponential averages (`Network::synchrony`), and the `sparkline` encoder for the braille membrane trace |
| `cortex.rs` | The Cortex module view: a neuron per process in a layered slice (seats by kind, widening in 1.25x steps), dendrites per thread, axons from `Snapshot::links` and `pipes` with their delays, the pulses in flight, the flashing somas, the dust of exits, the inspector lines and braille sparkline, and the raster panel drawn in screen space at the bottom of the frame; `Slice`, `Body`, `Axon`, `weight_of_rate` and `raster_bins` |
| `horizon.rs` | The Horizon module view: the OOM killer as a Schwarzschild black hole. Processes ride circular orbits in a lensed disk ranked by `oom_score` (rank 1 on the ISCO, `oom_score_adj` -1000 in a band far out; memory footprint where the platform has no score), and a rise of the OOM kill counter (vmstat, else the cgroups' sum) with a process vanishing sends the best-ranked victim on a plunge that freezes short of the horizon; the camera maps to inclination and azimuth, `Roster` and `kill_counter` read the sample, `Horizon::draw` draws back to front by layer, and the demo kill cycle is the row's `DemoOptions::oom_cycle` |
| `horizon/lens.rs` | Pure physics of the Horizon view, no drawing, in G = c = M = 1: `LensTable` (photon rays traced back from the observer by RK4 on d2u/dphi2 = 3u^2 - u, built once on first use, giving the primary and secondary image of a point), `Observer` (images of disk and sky points), `flux` (Page-Thorne), `one_plus_z`, `disk_emission`, `angular_velocity` and `period`, `PlungeTrack` and `plunge_brightness`; each ports a function of `docs/mockups/code/horizon.py` and is tested against analytic values |
| `horizon/orbits.rs` | The Horizon view's population with no drawing: `Ranking` (the swap margin), `Orbits` (bodies eased to the radius of their rank, ghosts of what vanished, the kill counter's rises that turn the best-ranked ghost into a plunge), `Ghost::pose`, `rank_radius` and `band_radius` |
| `horizon/disk.rs` | What the Horizon view draws as lensed triangles: the disk cut into a primary and a secondary image (cells about 4 px, at most 120k triangles, lensed again only when inclination or scale changes, recoloured every frame), `Turbulence`, `SkyField`, the stars, `draw_shadow`, `draw_ring`, `draw_dashed` and `draw_glare`, and the depth `layer`s |
| `murmuration.rs` | The Murmuration module view: one bird per thread in a single starling flock (`Flock`: the prototype's alignment over seven topological neighbours with same-process neighbours counting double, cohesion over 21, separation, the roost's pull, the falcon and its agitation wave, stepped at a fixed 20 Hz through `FixedStep` and `SpatialHash3::nearest`), the roster that thins the flock to `BIRD_CAP` (8,000) birds by the lowest hash of tid, birds that rise from and fall to the roost, the falcon released by memory pressure or an OOM kill (`Watch`: a view entered after a gap releases nothing), the flock drawn as stacked `Item::Puff` splats far to near with pick-only discs, and the treeline silhouette, which carries no data |
| `coop.rs` | The Coop module view: Vicsek flocks per cgroup, henhouses, nests (eggs for open files, brooding and queueing for file locks), feeders, chicks, dust and foxes, stepped at a fixed 20 Hz |
| `simulation.rs` | Shared pure simulation helpers: `SpatialHash` (2D) and `SpatialHash3` (3D, with exact k-nearest) neighbour queries, the seeded `Rng` (uniform and `normal`), `Complex` with `dft`, `fft` and `inverse_fft`, the `FixedStep` clock, hashes, and the `growth`, `sprout` and `smoothing` easing |
| `status.rs` | The legend wording for optional sources, so every view says it alike: `missing_note` (` \| no <source>: <consequence>`) and `sources_note` (` \| sources: a, b`) |
| `stellar.rs` | Stellar photometry for views that draw processes as stars: `magnitude` (from memory, `-2.5 log10(memory / 1 GiB)`, floored at 1 MiB), `spread` (core and halo size and strength from a magnitude and the frame height), `Frame::star` (one pickable pixel-sized sphere and at most one glow, existing items only), and the nova, fade and supernova timelines (`Flash`, `exit_flash`, `flash`), which depend only on age |
| `blackbody.rs` | Blackbody colours: `TABLE` (256 linear sRGB entries from 1000 K to 40000 K, geometric in temperature, regenerated by the ignored test `print_blackbody_table`), `linear` (clamped, interpolated in ln T) and `color(kelvin, brightness)` (sRGB-encoded `Color`) |
| `palette.rs` | Every process-kind, process-state and core-kind colour, `mix`, `tint`, `kind_color`, `core_color` and `core_name` |
| `geometry.rs` | `dot`, `normalize`, `normalize_or_zero` and the shaded `cuboid` |
| `picker.rs` | The view picker opened by `v` and the Tab playlist: `Picker` (filter, highlight, popup lines), the name and title filter, `playlist` (the `--views` rules) and the `cycle` and `toggle` rules; pure, so the tests drive it without a terminal |
| `layout.rs` | Stable placement: `Seats`, `Discs`, the golden-angle `spiral`, `vacant` and `spin` |
| `history.rs` | The shared per-process CPU and memory history: `History` (a ring of `CAPACITY` samples sorted by `Identity`, recorded by `Scene` on every sample whichever view is shown; `window` by `elapsed` time, `recent` and `series` by count, `changes` for the births and exits between consecutive samples), `Reading`, `Sample` (with an absolute `sequence`), `Change`. Views read it through `Stage::history`; `warm_up` replays it for headless demo runs |
| `medium.rs` | Pure simulation state for the Ripple (wave equation) and Flow (particles) views |
| `gpu.rs`, `shaders.wgsl` | wgpu backend that mirrors the CPU rasterizer; the field pass samples one storage buffer by hand (no hardware sampler) and `store_fields` rewrites only the slots whose field was shaded again |
| `terminal.rs` | Graphics and Kitty keyboard capability probe (the keyboard flags are pushed when the terminal answers, and `restore_screen` pops them before the alternate screen is left), frame transfer, text overlay, terminal restoration |
| `platform/macos/mod.rs` | macOS `Sampler` (libproc, Mach, sysctl, IORegistry, `proc_pid_rusage` V6 with fallbacks), the cluster clocks, the socket scan, the bounded open-file scan (`FileScan`, one `proc_pidfdinfo` per vnode), the pipe and FIFO scan (`PipeScan`: `PROC_PIDFDPIPEINFO`, and a vnode call per vnode descriptor to find FIFOs), the thread reader (`Threads`: `PROC_PIDLISTTHREADIDS`, then `PROC_PIDTHREADID64INFO` per thread), `oom_scores` (root only: `memorystatus_control`), and what it reports through `missing`, `per_cluster` and `unreadable` |
| `platform/macos/ffi.rs` | Every extern declaration and `#[repr(C)]` struct that `libc` lacks, with size assertions; `memorystatus_control` and the jetsam priority entry |
| `platform/macos/logic.rs` | Pure macOS decisions with no FFI: jetsam priority lists (`jetsam_priorities`) and their rank (`jetsam_oom`), kinds, groups, parents, the Mach scheduling policy, `KERN_PROCARGS2` parsing, tick conversion, cluster clocks from cycle counters, socket pairing and queues, open and deleted-but-open files and pipe ends from `vinfo_stat` and `pipe_info`, thread run states and ids |
| `platform/macos/journal.rs` | The unified-log follower (`log show`, then `log stream`) and its std-only JSON line parser |
| `platform/macos/events.rs` | The `isotop-events` thread on macOS: exec and exit of the sampled processes from kqueue (`EVFILT_PROC`) |

`agents_file_table_lists_every_source_file` fails when a `.rs` file under `src/` has no row. Name
files by path relative to `src/` in the first column; a row may list several.

## Architecture

Dependencies mostly run one way: `platform` and `model` (the data), then the shared helpers, then
`render` with `views` and the view modules, then `gpu` and `terminal`, then `main`. Check with
`grep -n 'use crate::' src/*.rs` before adding an import. Nothing imports `main.rs`, `model` and
`platform` never import `render`, a view or `terminal`, and `gpu` and `terminal` import only
`render`. The exceptions, so don't assume otherwise:

- `model` and `platform` import each other: the platform builds `model` types and `model` calls the
  `Sampler`. `journal` and `platform/*/journal.rs` do the same,
  and `model` imports `journal::Delivery` for `Snapshot.journal`.
- `render` imports no module view's state or drawing, only the registry (`views`: `REGISTRY`,
  `ViewInfo`, `ViewModule`), the `Orbit` ellipse that `Frame::trace` strokes (`views::systems`),
  `globe::drift` for the camera's turn of the earth and the `glyphs` bitmap font; each view imports
  `render` for `Frame`, `Stage` and `Color`. `render` and `views` import each other: `Scene` draws the rows, and the rows name
  `Scene`, `Stage` and `Backdrop`. `palette`, `geometry` and `medium` import `render` too, for `Color`, `Point` and
  `Frame`. `simulation` and `layout` import only `model::Identity`.
- The test-only `harness` imports `main` for `App`, `Options`, `demo_snapshot` and `warm_up`, so
  it runs a view exactly as a headless `--output` run does.
- `render` and the views read `platform` types (`Shadow`, `Files`, `DeletedFile`, and `Source`
  and `Sources` from `Snapshot::missing`), and Cells and the coop its `GROUP_LABEL` and
  `WITHOUT_CGROUPS`.

**The view contract.** A view is one module plus one row of `views::REGISTRY`. The module owns
a state struct and implements `views::ViewModule`:
`draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize` and
`legend`, and where it needs them `record` (every sample, whichever view is shown), `key`,
`inspector_line` and `card` (the inspector card of a `Thing` it drew, built on demand). A view
reads only `Stage`, the process slice and `&Snapshot`, and talks back only through `Stage`
outputs. `Stage` carries in the frame, camera, selection (and `thing`, the selected or hovered
thing), the process the tour visits (`toured`), the focused subtree's root, `--limit`, the `c`
link mode and the processes whose links stand out (`links`, `highlight`, for a row whose links
rule is `DrawnByView`), time, the `frames` the scene has rendered (a view that must tell the next
frame from one after a gap), `dt`, births and growth, the `splashes` where processes left and the
`newborn` that arrived (the disturbances of a medium), and `history`, the shared per-process CPU and memory
history (`history.rs`), which `Scene` fills on every sample whichever view is shown and a view
reads from the shown snapshot's `elapsed` (`History::window`, `History::series`); what the
platform could not measure is `snapshot.missing`, which `Stage` does not
copy. `stage.dt` is the one step policy for new code, computed once per frame by `Scene`: 0 on
the first frame, while paused and when time goes back (a rewind), otherwise the time since the
last frame that moved forward, at most `render::MAX_DT` (0.5 s). Step a simulation with it or
with a fixed-step clock fed from `stage.time`; the older views keep their own clamps. A view
writes back `stage.positions` (picking anchors, labels, the tour and the exit flash all read it),
`stage.bounds` (a fixed envelope for the camera fit when the content moves by itself),
`stage.headroom` (how high content stands above the ground, for the fit of a view that gives no bounds),
`stage.places`, `stage.titles` (the names of systems, labelled under their outer edges),
`stage.notes` (popup lines after the sockets), `stage.sparklines` (generated
braille lines right after them, cleaned by `terminal::clean_sparkline`, never process text),
`stage.tour` (`TourTarget { id,
position, weight }`, which the tour visits heaviest first) `stage.popup` (popup lines right
after the process's own figures) and `stage.callout` (lines for the tour's callout of a process), and returns how many processes it showed; the rest count as
collapsed. `Scene::render` takes one `RenderInput` (snapshot, view, camera, size, selection,
time, limit, focus and whether the app is paused). The row (`ViewInfo`) holds
everything else that sets the view apart, and `Scene`, `App` and the headless warm-up read it
instead of naming the view. There is one way to write a view: a module behind
`ViewModule`, built by its row's `build`. `Scene` holds only frame-wide state and has no
per-view field, branch or arm (`scene_has_no_view_specific_fields` does not compile until a new
field is listed in it, and a field needs a doc comment saying why it is frame-wide), so a view's own state goes in its module, never on `Scene`. `Scene` builds every
module when it is created and feeds each one `record` on every sample, so a view that is
not shown still has its history when it opens.

**Shared helpers live in one place each.** Don't write a second copy:
- `simulation`: `SpatialHash` and `SpatialHash3`, `Rng` (seeded per identity, `normal` for a
  Gaussian), `XorShift32`, `Complex` with `dft`, `fft` and `inverse_fft` (forward sign
  `exp(-2*pi*i*k*n/N)`), `FixedStep` (a fixed-step clock: `advance(time, tick)` runs whole steps
  and returns the interpolation fraction; step simulations with it, never copy `Coop::advance`),
  `simulation::hash` (`fnv1a_64_bytes` and the rest), `hash_identity`, and
  the `growth`, `sprout` and `smoothing` easing.
- `blackbody`: `color(kelvin, brightness)` for anything drawn as incandescent; never a second temperature-to-colour table.
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
  `docs/VIEW_CONVENTIONS.md` (`CPU/GPU parity`). Stacked `Item::Glow` items drift apart (the CPU blend truncates where the GPU's rounds), so content whose look is the density of many splats draws `Item::Puff`, which rounds on both.
- **Layouts are stable.** A process keeps its place when CPU, memory or the population changes.
  Systems and districts are only re-placed when their structure outgrows the reservation, and
  tests enforce this: the view harness checks every registered view (`harness.rs`). Don't
  introduce a layout that reshuffles every sample.
- **Picking.** A pickable item carries an index into `frame.identities`, or a thing pick from
  `Frame::thing_pick` (an index into `frame.things` with the `THING` bit set) for something that
  is not a process. `NONE` means "not pickable", and lines clear picking where they draw.
  `Item::Pick` writes picks only, for blended content, and `Item::Field` writes the owner of
  the nearest node of its field (`NONE` without an owner grid). Mouse input is cell-coarse, so
  `target_near` (and `pick_near`, processes only) searches a radius. Never mint a synthetic
  `Identity` for a thing.
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
  terminal (`process_text_cannot_inject_terminal_controls`). Keep it that way for any new text:
  thread names (`Thread::name`) are stored raw and reach the terminal only through
  `terminal::clean_text`.
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
pause, the Space visits, quit and terminal restoration.

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

## The wiki

`wiki/` holds the sources of the GitHub wiki: theory of operation, the mathematics behind each view, a page for
each view and planned view, guides and media. `.github/workflows/wiki.yml` checks it with
`python3 scripts/check_wiki.py` and publishes it on every merge to `main`, so edit the files, never the wiki UI.
A PR that changes what a view does or how the pipeline works updates the page that describes it (cite files and
symbols, not line numbers); `wiki/Guide-Contributing-to-the-Wiki.md` has the conventions, including the figure
scripts in `docs/figures/` and the rule that a planned view's page says it is a design. Wiki-only changes use a
`docs/...` branch.

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
    `ViewInfo::new(name, title, backdrop, || Box::new(<Name>::default()))`, setting only the fields that
    differ from these defaults: `available` (everywhere; a runtime probe lives in `platform/`),
    `warmup` (none; a new view's phases run under `When::Headless`, and a phase that replays
    demo samples, such as `Phase::Replay` at `--sample-ms` or `Phase::Play`, which also draws the frames between them, under `When::DemoHeadless`), `links` (`Quiet`; or `All`,
    or `DrawnByView`), `link_strokes` (off: the scene's links are beams; a new view turns it on,
    so its links are hairlines that rasterize alike on the CPU and the GPU), `link_lift` (0: how
    high the scene lifts its links above the drawn positions, 1.5 over City's rooftops), `flash` and `haze` (on), `pitch` (None: leave the camera's), `tour` (on),
    `header` (None: the scene's visible and collapsed counts), `face` (None), `turn` (None: the
    wheel pans), `keys` (none: the view's own keys, which lead the `?` help line while it is
    shown), `sources` (none), `demo` (`DemoOptions::NONE`; a row names only the options it turns on, `DemoOptions { churn: true, ..DemoOptions::NONE }`, so a new option leaves the other rows alone) and `contract`
    (`Contract::DEFAULT`: it draws processes, which may move at most 1 world unit over the
    ground, checked where they are drawn at 12 s of demo time, and it is known to fail nothing).
    Tab, Shift+Tab, the `v` picker, `--view`, `--views`, `--list-views` and `--verbose` follow
    from the table. Never reorder the first ten rows: the digits pick them.
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
    the row's `contract`, and a later `time` there if its history window is not full at 12 s;
    a module view whose bodies move by themselves returns the places they keep from
    `ViewModule::anchors` (as Coop's seats). Never
    add a `known_unstable` entry for a new view: those record debt of the views that predate the
    harness, each with its issue, and the harness reports one that starts to pass.
  - Test the module through `Scene::with_rows` and a table of its own, as `views::testing` does.
  - Outside `src/`: the README section, key table and view lists, and a line in
    `tests/golden.txt` (bless it and say so in the PR). CI renders every view `--list-views`
    prints, and the golden test renders every row.
- Prefer the standard library. Add a dependency only when it replaces substantial,
  error-prone code.
- Suppress lints with `#[expect(lint, reason = "...")]` only, never `#[allow]`.
- A view with a page in `docs/theory/` has its constants and function names quoted there. Change them together, check any claim you touch against `src/`, and run `python3 scripts/code_report.py` (Markdown counts in the report). `docs/theory/figures/make_figures.py` regenerates the page diagrams; it needs matplotlib and numpy, which no gate does.

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
