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
   them: `main.rs` (at startup) and `interactive.rs` (on every view change) hand `Collector::demand` the `sources`
   of the shown view's registry row, the thread takes a copy of that `Demand` at the start of
   each cycle, and reads an optional source through `Demand::read_if` (or `Reads::read`, which
   also records it as active or, when it cannot be read, missing). Remote locations
   (`Source::Locations`, the GeoIP lookups the globe draws) and pipes (`Source::Pipes`, the
   descriptor walk and fdinfo reads behind `Snapshot.pipes`) go through it; the unread bytes of
   Unix sockets (`Snapshot.unix_queues`) ride the socket scan, which every view reads, and are not
   demanded. The journal follower (`interactive.rs`, scheduled by `app/feed.rs`) runs while the demand contains `Source::Journal` and for
   `JOURNAL_LINGER` (30 s) after; no view is special-cased. While the demand contains it, `App::feed_journal` attaches
   each frame's new lines (the follower's, or the demo's) to the newest sample as `Snapshot.journal`
   (`journal::Delivery`, numbered so a view takes each line once), and the view that draws them reads
   them from its snapshot (a `journal::Line` carries its syslog priority and, on Linux, the `JobResult` of the job it ends, from the
   `JOB_RESULT` field; `Source::JobResults` is the gap macOS reports). In demo mode a row's `DemoOptions::job_results` makes the
   feed `journal::demo_spirits`, the lines that end a job and a thin share of `journal::demo`, in place of `journal::demo`. A sample that stops being the newest (`App::push_sample`) lets go of its
   lines and keeps their numbering, so the history holds one sample's lines (at most 2048), not
   `--history` times that. The kernel counters (`Source::Interrupts`, `Softirqs`,
   `IrqAffinity` and `OomKills`; `platform/linux/counters.rs`, `model/kernel.rs`) go through it
   too: with none demanded no `/proc` counter file is opened, and `Snapshot.interrupts`,
   `softirqs` and `oom_kills` stay Pending, Pending and None. The foreground samples between two passes carry the same readings, so `Snapshot::kernel_pass` numbers the pass (`KernelReadings::pass`; the demo uses the sample's time) and a view that differences or ages them takes each pass in once. Threads (`Source::Threads`) are read
   the same way: while they are demanded each sample hands the thread the processes busiest
   first (`threads::targets`, CPU descending, ties by `Identity`), and a pass
   (`threads::ThreadScan`) walks that list, reading at most `THREADS_PER_PASS` (8,192) threads
   and `THREADS_PER_PROCESS` (1,024) per process and never sorting on its own. A process cut
   short, or not reached, has a `thread_list` that is not `complete`. The I/O detail
   (`Source::IoDetail`, `io_detail.rs`) is read for the `IO_PROCESSES` (24) busiest processes
   only: the frame thread picks them with `io_detail::targets` (largest `platform::io_rank`
   first, ties by `Identity`: Linux ranks by syscalls per second, macOS, which counts none, by
   disk I/O per second), and a pass (`io_detail::IoScan`) reads each one's regular files with
   their descriptor positions through `platform::Descriptors`, at most `FILES_PER_PROCESS` (256)
   files per process and `DESCRIPTORS_PER_PASS` (4,096) descriptors per pass, setting
   `IoDetail::partial` when a cap cuts it. A file is credited with the growth of a descriptor's
   position between two passes, and only that: a descriptor that is new, reopened, rewound or
   changed in its flags, one opened `O_APPEND` and one past the end of its file claim nothing
   (`Flow` says which), and I/O that moves no position (`pread`, `pwrite`, mappings, pipes,
   sockets) stays in `IoDetail::unattributed`. Where the platform counts the bytes of read and
   write calls (Linux `rchar` and `wchar`, read just before and after the walk) the claims of a
   process are held to the growth of those counters; where it does not (macOS)
   `Source::UnattributedIo` is missing, `unattributed` is None and nothing bounds a claim but
   the file's size. A process not among the busiest, or not reached, has `io_detail` Pending.
   The pipe scan also gives each process what it holds of pipes (`Snapshot.pipe_holds`, a
   `PipeHold` of read and write ends) and the pipes whose write end is held and whose read end no
   process isotop read holds (`Snapshot.severed`; `model/pipework.rs`); loopback TCP is rated per
   direction as well as per pair (`Snapshot.loopback_sent`, a `Hose` of rate and connections, from
   the received counters of each end). Where a process that holds pipes sleeps in the kernel
   (`Source::WaitChannel`, `platform/linux/wait.rs`, `/proc/<pid>/wchan`) is read for at most
   `WAIT_PROCESSES` (256) of those processes per pass, round-robin, and kept per `Identity` like
   the open files: `Snapshot.waits` holds a `Wait` (writing to a pipe, reading from one, anything
   else, or `wchan` reading 0). macOS has no wait channel and `Source::WaitChannel` is missing there.
   `Snapshot.sources` lists the
   optional sources read for the sample, and `Snapshot.missing` those demanded and unreadable;
   one nobody demanded is in neither. `--demo` starts no thread, so `sources` is empty there.
   While the demand contains `Source::LifeEvents`, `Collector::sample` also runs a second thread,
   `isotop-events` (`platform/linux/events.rs`: the proc connector and taskstats;
   `platform/macos/events.rs`: kqueue), which fills a bounded `LifeQueue` that each sample drains
   and merges with sample diffing into `Snapshot.life`; dropping the queue when the demand ends
   stops it. On Linux the sampler caches each process's cmdline and cgroup per `Identity` and re-reads them after an
   exec or every few samples, so the frame loop reads only `stat` and `io` per process.
   `app/player.rs` (`HistoryPlayer`) keeps a history ring of snapshots for pause and rewind.
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
   text over the image, and are made of `SafeText` only.

| File | Owns |
|---|---|
| `armorial.rs` | The Armorial module view: house arms generated from the path hash, measured cadency, counterchanged tinctures and blazon, reserved bays and seats, pickable cloth fields, kind helms and mourning hatchments |
| `main.rs` | `main` and `run` (`preflight`, then a headless or an interactive run), `start` (the first sample, the collector, the journal follower and the `App` a run begins with) and `demo_snapshot` (every demo sample goes through it with the shown view's `DemoOptions`, and the time of the sample before, for the life events in between). It re-exports `App`, `Options` and `warm_up` for the test-only harness; the golden render test is in `headless.rs` |
| `cli.rs` | `Options` (every flag), `RendererChoice` and `Power` (the values of `--renderer` and `--gpu-power`), and `preflight` (`--verbose`, `--list-views`, `--keys` and views this platform lacks, before the terminal is touched) and `parse`, which ends `--help` with the controls from the keymap |
| `headless.rs` | `--output` and `--benchmark`: `run` draws the frame after `warm_up`, which replays the shared history (headless demo runs, unless the row's `Phase::History` fills it) and runs the shown view's warm-up phases before the frame (the golden render test calls it too); `benchmark` times `App::draw` (record) and `App::rasterize` (raster) apart and `Timing::report` prints both |
| `interactive.rs` | The live run: `Live` owns the terminal, the layout and the clocks, and `Live::run` takes one turn per frame as `sample`, `follow_journal`, `render`, `present`, `input` and `finish` |
| `app/mod.rs` | `App`, the interactive state (the scene, the shown view, the selection and focus, and the components below), `push_sample` and `take_sample`, `steer`, `render` (`draw`, which records the frame in two steps when the camera snaps to a fitted goal, then `rasterize`), `touch`, `panel_shown` and `demand` |
| `app/camera.rs` | `CameraController`: the camera, its goal and the camera that drew the last positions (`rendered`), `approach` (the easing), the fit-on-growth flag and the `framed` flag (`fit_to`, `follow`, `refit`), `reset` and `zoom_to_visit`; `App::release_globe` and `App::face` |
| `app/player.rs` | `HistoryPlayer`: the ring of snapshots, the pause and the shown sample (`shown`, `push`, `toggle_pause`, `rewind`, `forward`) |
| `app/search.rs` | `Search`: the query, the matches and the current match, with the pure `matching` filter; `App::search_key` (which reads the `Action` the keymap gave) and `select_match` |
| `app/pointer.rs` | `Spot` (a mouse report's frame position and cell) and `Pointer` (hover, hovered process and thing, drag, dismiss), `App::mouse` and `pointer_event` |
| `app/tour.rs` | `Tour`, `Step` and `Cursor` (the Space visits share one cursor over `visit_cycle`), `Candidates` (the tour's targets, cycle and points as a pure function of the frame), `update_tour`, `visit`, `TOUR_STEP` |
| `app/input.rs` | `App::key`, `command` (looks the press up in `keys` and offers the shown view its key hook) and `perform` (the global actions), `typed` (the key as legacy terminals would have typed it) and `view_key` (what the shown view's key hook is offered) |
| `app/switcher.rs` | Changing views: `App::switch`, `cycle`, `entries`, and the picker's keys (`picker_key`, which reads the `Action` the keymap gave) and popup (`picker_popup`) |
| `app/overlay.rs` | The text drawn over the scene: `App::overlay` (labels, titles, the hover tag, the tour callout), `popup` and `thing_popup` (the inspector) |
| `app/status.rs` | The status panel's lines (`App::text`) and their helpers (`known_bytes`, `pressure_text`, `unreadable_text`, `sampler_text`) |
| `app/screen.rs` | `Layout` (the frame and the terminal's cells), `Board` (cells already claimed by panels and labels), `place_panel` and `place_lines`, `PANEL_ROWS` |
| `app/untrusted.rs` | Test only: every string the system supplies replaced by hostile text (`HOSTILE`), every view drawn with a selection, hover, tour and each thing opened, and the terminal output searched for what a terminal would act on |
| `app/renderer.rs` | `Renderer { gpu, kind }`: the choice of rasterizer, `rasterize` for a recorded frame on the GPU or the CPU, and the fall back to the CPU when the GPU fails |
| `app/feed.rs` | `JournalFeed`: when the journal follower runs (`JOURNAL_LINGER`) and the numbering of the lines `App::feed_journal` attaches to the newest sample |
| `app/fixtures.rs` | Test only: the helpers (`all_available`, `press`, `probe_app`, `guided_app`, `globe_at`, `options`, ...) that the tests of the `app/` files share |
| `model.rs` | `Process`, `Snapshot`, `Collector` (CPU smoothing, per-interval rates and shares from counter deltas, background thread, merge), `Demand` and `Reads` (which optional sources the background thread reads), `top_n` (the budgeted selection: largest key first, ties by `Identity`, NaN last), `Life` (merges the event thread's observations with sample diffing into `Snapshot.life`: one exit per `Identity`, never revised), `Oom`, `Rollup` and `ROLLUP_PROCESSES` (the 64 largest processes get `Process::memory_detail`, read in a 50 ms budget that resumes where it stopped), `MemoryReads` (the two memory sources of the background thread) and `Kept` (readings kept between cycles per `Identity`), `read_io_detail` (the `Source::IoDetail` pass, which also makes `Source::UnattributedIo` missing where the reader counts no bytes) and `Process::io_detail` (`Measured<Arc<IoDetail>>`; the frame thread hands the background thread `Collector::io_targets`, chosen by `io_detail::targets`), demo workload (`demo_with` with the `DemoOptions` a view's row asks for: `churn` for deterministic births and exits, which `demo_life` lists and `demo_events` turns into life events, `exact_cpu_time`, `observer` for `Snapshot::observer`, isotop itself, `mortality`, which shapes the churn's deaths for a graveyard (every cause, a burst, a stopped unit, a reused pid), `oom_and_memory_detail`, which gives every process an `Oom` (three System processes exempt) and the 64 largest a consistent `Rollup`, one of them unreadable, and `kernel_counters`, which synthesizes `Snapshot.interrupts`, `softirqs` and `oom_kills`, the last the sum of the demo units' `oom_kills`, and `threads`, which gives every process the threads `threads::demo_list` synthesizes, and `job_results`, which makes the demo journal one with job results (`app/feed.rs` reads it; no snapshot field changes); plain `demo` leaves all of these unread; `channels`, which synthesizes `Snapshot::unix_queues` and `Snapshot::pipes`; `plumbing`, the boiler room (`model/pipework/demo.rs`: pipes with `Pipe::id`, `pipe_holds`, `severed`, `waits`, `loopback_sent`, queues and the `io_detail` of the cast); `io_detail`, which gives the base processes among the 24 busiest by syscall rate a `Process::io_detail` that `io_detail::demo_detail` makes with the real attribution maths (`demo_syscall_chars`, the exact integrals of their syscall bytes, are what its counters and positions advance with); `oom_cycle`, the out-of-memory kill every 30 s; `early_giant`, which with `churn` moves the first one-GiB exit to 8.8 s; `exchange`, which replaces `kernel_counters`' interrupt table with `model/exchange.rs`'s; `DEMO_EPOCH`, the wall clock of demo time 0; `demo` is the unoptioned test shorthand), `Queue` and `Pipe` with `pair_pipes` (pairs the pipe and FIFO descriptors of every process by `PipeId` into reader and writer links), `describe` |
| `model/exchange.rs` | The demo's interrupt table for the Switchboard (`DemoOptions::exchange`, which replaces `kernel_counters`' smaller one): the prototype's 48 numbered lines on 16 CPUs at about 41,000 events a second, each with its effective affinity, `eth0-rx-2` moving between cpu3 and cpu9 at 10.2 s and every 30 s after (a sample a second apart first sees it at 11 s, so `--time 12` is a second into the swing), one NMI on cpu0 every 45 s from 10.7 s, `LOC`, `RES` (proportional to the CPU's busy share), `CAL` and `TLB`, and all ten softirqs; pure functions of the time, built on `kernel`'s waves |
| `model/kernel.rs` | The kernel counters of the background thread: `KernelScan` (one pass: reads each demanded source through `Reads`, joins the affinity onto the interrupt rows, turns counts into per-CPU rates from the change since the previous pass, None on a first pass, a new row or CPU and a count that went backwards), `timer_row` (`LOC`, or the `arch_timer` row on arm64), `Interrupt::is_numbered`, and the demo's synthesized interrupts and softirqs (`demo`, pure functions of time) |
| `model/pipework.rs` | What the plumbing view reads of pipes and sockets beyond the shared fields: `PipeHold` (the distinct pipes and FIFOs a process holds a read end and a write end of, a FIFO opened for both counting in both), `Severed` (a pipe a process holds a write end of and no process the scan read holds a read end of, at most `PIPE_FAN_OUT` writers each), `Hose` (the rate one process sent another over loopback TCP and over how many connections), and `roles`, which derives the first two from the descriptors `pair_pipes` takes |
| `model/pipework/demo.rs` | The boiler room `DemoOptions::plumbing` synthesizes over a finished demo snapshot (`apply`): sixteen base processes renamed and resized into the cast (`CAST`) in four cgroups (`BAYS`), their pipes, a FIFO both ways, Unix sockets with queues, a loopback connection, wait channels and the `IoDetail` a pass would have read, and a pipe whose reader lets go for eight seconds of every twenty (`DROPS_AT`, `RETURNS_AT`); a demo of more than 192 processes (`EXTRA_FROM`) strings the rest into pipelines of four, so `--processes 1000` reaches the view's caps; pure functions of the time |
| `platform/mod.rs` | The OS contract (`Sampler`, `RawProcess`, `network`, `account`, `FileScan`, `locks`, `Gpu`, `open_events`, `journal`, the kernel counters `interrupts`, `softirqs`, `irq_affinity` and `oom_kills`, and `oom_scores`, `rollup` and `Taken`, one reading of a process that may be gone or unreadable), the `Source` registry and `Sources` bit set that `Snapshot::missing` holds, the platform-neutral `Network` (with the Unix socket queues), `Files`, `Locks` and the pipe types (`PipeScan`'s `PipeSample` of `PipeEnd`s, `PipeId`, `AccessMode`), `Descriptors` (the `io_detail::DescriptorReader` each platform provides) and `io_rank` (how it ranks the processes to read), `WaitScan` and `Wait` (where a process that holds pipes sleeps), and the life event types (`LifeEvent`, `Cause`, `ExitRecord`, what an event source `Observed`, and the `LifeQueue` it fills); selects the implementation by `target_os` |
| `platform/linux/mod.rs` | Linux `Sampler`: `/proc` parsing, CPUs, pressure, core kinds, cgroup v2 accounting, and the gaps it reports through `missing` (cpufreq, schedstat, core kinds, cgroup v2) |
| `platform/linux/net.rs` | Socket links: `/proc/net/tcp*`, sock_diag netlink (Unix peers with their receive queues from `UNIX_DIAG_RQLEN`, inet TCP with `tcp_info`) |
| `platform/linux/memory.rs` | The out-of-memory ranking (`oom_score`, `oom_score_adj`) and one process's `smaps_rollup` in bytes, with their parsers; kernels before 5.3 have no `Pss_` split, which is left unknown |
| `platform/linux/files.rs` | Open regular files and deleted-but-open files per process (`/proc/<pid>/fd`, bounded per scan) and file locks with their waiters (`/proc/locks`); `classify` names what a descriptor's link is as a `Target` (a `Regular` file with its path, size and whether its last link is gone, a pipe, socket, terminal, `anon_inode` or other), which the open-file scan folds into its private `Descriptor` and `descriptors.rs` reads whole |
| `platform/linux/fdinfo.rs` | `/proc/<pid>/fdinfo/<fd>`: `parse_fdinfo` (flags in octal, offset) and the access mode from `O_ACCMODE`, `FdInfo::appending` (`O_APPEND`), shared by the pipe scan and per-file attribution; fixtures in `platform/linux/fixtures/fdinfo_*.txt` are real captures |
| `platform/linux/descriptors.rs` | The Linux `DescriptorReader` (`Descriptors`), behind `Source::IoDetail`: one process's regular files with access mode, position and size (`fd` and `fdinfo`, `Budget` caps on descriptors examined and files kept), its other descriptors counted by `Target`, `rchar` and `wchar` of `/proc/<pid>/io` read just before and just after the walk, and the pid's start time checked afterwards; `io_rank` (syscalls per second) |
| `platform/linux/pipes.rs` | `PipeScan`, behind `Source::Pipes`: `pipe:[inode]` descriptors and FIFOs (a stat), direction from fdinfo, at most 4096 fdinfo reads per pass round-robin by pid |
| `platform/linux/nvml.rs` | NVIDIA per-process GPU memory via `dlopen`; never wakes a runtime-suspended GPU |
| `platform/linux/journal.rs` | `journalctl` in export format and its parser, which keeps `JOB_RESULT` as `Line::job` |
| `platform/linux/counters.rs` | `/proc/interrupts` (columns follow the `CPUn` header, numbered and named rows, totals for rows with fewer counts than CPUs), `/proc/softirqs`, `effective_affinity_list` per numbered IRQ, `parse_cpu_list`, and `oom_kill` from `/proc/vmstat`; fixtures in `platform/linux/fixtures/`, real captures where the header says so |
| `threads.rs` | Per-thread sampling (`Source::Threads`): `Thread`, `ThreadList` (in `Process::thread_list`), the caps, `ThreadReader` (what each platform's `Threads` provides), `DeltaTracker` (cumulative counters to growth, forgetting keys a pass missed) and `ThreadScan` (the capped pass and per-thread CPU keyed `(tid, start)`), `targets` (the order the frame thread hands over) and `demo_list` (the demo's threads, `DemoOptions::threads`) |
| `io_detail.rs` | Per-file I/O attribution (`Source::IoDetail`): `IoDetail` (in `Process::io_detail`: `files` of `FileFlow`, counts of pipes, sockets, ttys, anonymous and other descriptors, `unattributed`, `partial`), `Flow` (`Unmeasured`, `Unpositioned` or `Measured` read, written and either rates), `Unattributed`, the caps (`IO_PROCESSES` 24, `FILES_PER_PROCESS` 256, `DESCRIPTORS_PER_PASS` 4096), `DescriptorReader` (what each platform's `Descriptors` provides: a `Held` of `OpenFile`s, `Counts` and the `Characters` bracketing the walk, within a `Budget`), `IoScan` (the capped pass, remembering one reading per `Identity`), `select` and `targets` (the busiest by `platform::io_rank`, on the frame thread), `by_syscall_rate` |
| `io_detail/attribute.rs` | The attribution maths, pure: `attribute` and `unmeasured` turn two `Held` readings, the seconds between them and a `Limit` (`Unbounded`, `To` the growth of `rchar` and `wchar`, `Withheld`) into the files' flows and the unattributed remainder; a descriptor is keyed by (fd, device, inode, access mode, `O_APPEND`), copies of one open file description count once, and the integer scaling never claims more than the counters grew |
| `io_detail/demo.rs` | `demo_detail`: the demo's `IoDetail`, made by `IoScan`'s own `detail` from two synthesized readings one `BACKGROUND_PERIOD` apart, files under `/usr/lib`, `/home/demo`, `/var/log` and `/tmp` (one deleted, one appended to) whose positions advance with `demo_syscall_chars` |
| `platform/linux/threads.rs` | The Linux `ThreadReader`: `/proc/<pid>/task/<tid>/stat`, parsed after the last `)` with the field indices `parse_stat` uses (`split_stat`, `timing` in `platform/linux/mod.rs`) |
| `platform/linux/wait.rs` | `WaitScan`, behind `Source::WaitChannel`: `/proc/<pid>/wchan` of the processes that hold pipes, at most `WAIT_PROCESSES` a pass round-robin by pid, `parse` (the kernel function names for a task blocked in a pipe write or read, kept as `Wait::PipeWrite` and `PipeRead`, `0` as `Unreadable`, any other as `Other`; the symbol itself never leaves the file), None when the kernel has no `wchan` |
| `platform/linux/events.rs` | The `isotop-events` thread on Linux: proc connector fork, exec and exit events and taskstats exit records over netlink, decoded by byte offset from the uapi headers, thread records summed per process |
| `journal.rs` | Journal lines for the Matrix and Ouija views: `Line` (priority, text, `JobResult`), the reader thread, bounded backlog, demo lines (`demo`; `demo_jobs`, a job line every 7 s; `demo_spirits`, the merge Ouija's row asks for), and `Delivery`, the lines one sample carries on `Snapshot.journal` |
| `views.rs` | The view registry: `REGISTRY`, one `ViewInfo` row per view in Tab order; `View`, an index into it; `Build`, the constructor a row holds for its module; `ViewOptions`, the command-line settings (`--hr-diagram`) that `Scene::configure` hands every module once, through `ViewModule::configure`, before its first frame; the `ViewModule` trait, `ViewKey`, `Warmup`, `LinkRule` and the harness `Contract` (`Property`, `Unstable`); and `testing`, module views for tests |
| `views/city.rs` | The City module view (declared from `views.rs`): `City` keeps each process's plot (`Plot`: a district and one of its 16 slots, taken from the first vacant slot of its cgroup's districts and kept until the process leaves the sample), `district_origin` spirals the districts out in square shells without moving any, and a building is its height from CPU, footprint from memory, windows, rooftop lamp, GPU beacon and I/O pulse; it reports the tallest height in `Stage::headroom` for `Scene::fit`, and its row's `link_lift` of 1.5 raises the links over the rooftops |
| `views/systems.rs` | The process tree as nested Keplerian systems, which the Orbit and Flow module views share: `layout` (the trees laid out for one frame, as a `Layout` of `Path`s, `Body`s and `Star`s), `Tree`, `Walk`, `hubs` (a hollow `Process` stand-in for a parent isotop cannot read, with `HUB_START` identities that never equal a process's, so it is never pickable), `Galaxy` (each of Orbit and Flow owns one: the layout code is shared, the state is not, so switching from one to the other never shows wells at the places Orbit has turned its systems to; `place` reserves each system's room, with `HEADROOM` to spare for changes of memory and population, and keeps its place until the structure outgrows it, `revolve` turns the systems about their barycenter in Kepler bands, only across consecutive `Stage::frames`), `shells`, `elements`, `period`, `Orbit`, `system_label` and `hub_note`; `Star::at` is where a system's star stands (a hub is no process, so `Stage::positions` has none for it and the camera fit takes its place from its `Title`), and `Layout::report` tells the app the tour stops (`Stage::tour`, a hub left out), the system titles (`Stage::titles`) and the memory a system holds (`Stage::popup`), and `Layout::homes` (test only) gives each process its system's centre, the place the two views keep, for their `ViewModule::anchors` |
| `views/orbit.rs` | The Orbit module view (declared from `views.rs`): `Orbits` keeps a `Galaxy`, lays the systems out with the orbital phase following the scene's time, and draws the orbits, CPU glows and trails, the bodies (a hub as an outline), GPU halos and thread rings; its legend adds `gpu_note` and `systems::hub_note`, and its anchors are each process's system centre, since the orbits and slots inside follow memory and the other bodies; where a body stands inside its system is not yet stable under changes of memory and population, tracked in [#142](https://github.com/tfpickard/isotop/issues/142) |
| `views/flow.rs` | The Flow module view (declared from `views.rs`): `Wells` keeps a `Galaxy` and a `medium::Flow` sheet, lays the systems out with the bodies held still, and pours the weather into the sheet (`weather`: a bowl per system and a dimple per heavy process for memory, whirlpools for CPU, two-lane `Lane` rivers for sockets, sparks for connections leaving the machine), kicks the particles from `Stage::splashes` and `Stage::newborn`, and draws the fabric, the bodies sunk into it and the particles; its anchors are each process's system centre, the well it sinks into; where a body stands inside its system is not yet stable under changes of memory and population, tracked in [#142](https://github.com/tfpickard/isotop/issues/142) |
| `views/ripple.rs` | The Ripple module view (declared from `views.rs`): `Ripple` keeps the `medium::Wave` and the pebble `Cluster` of each cgroup (a sunflower spiral, a slot kept while its process lives, drawn or not; pebbles grow in place into the clearance left round them, so a cluster is spread out only when its largest would touch a neighbour, which no doubling of memory brings about), drives the wave from each busy process's own pitch and from the scene's births and exits (`Stage::newborn`, `Stage::splashes`), lights the water as facets and gives each patch to its nearest process (`territories`, which allocates its owner list and queue every frame); a cluster of two or more drawn pebbles is a system, named in `Stage::titles`, `Stage::tour` and `Stage::popup`. It draws with `render::cover` (the grid over the layout), `similar` and `glows`, which Flow shares |
| `harness.rs` | Test only: the shared view harness. `Session` runs a view headless through `warm_up` and `App::render`; `rescaled` and `churned` perturb a demo sample; `check` tests every `Property` of a row against its `Contract`, and `every_view_satisfies_the_view_contract` runs it for every registered view |
| `render.rs` | The module root of `render/`: the `Color` and `Point` aliases, the shared colours and `MIN_BODY_PX`, `Links`, the test-only `hiding` checks, and the re-exports that keep `crate::render::{Frame, Camera, Item, Scene, ...}` working for every caller |
| `render/camera.rs` | `Camera` (projection, pan, zoom, tilt, `ground`, the globe's turn and hold) and the constants `SCALE`, `ISOMETRIC`, `LOWEST_PITCH` and `ORIGIN_Y` |
| `render/frame.rs` | The display list and what a frame owns: `Backdrop`, `Sky`, `Thing`, `Target` and `Card` (non-process pickables and their inspector cards), `Item`, `Buffers`, `Frame` (construction, `release`, the pick buffer queries `thing_pick`, `target`, `pick_near`, `target_near`, `locate`) and `write_png` |
| `render/raster.rs` | The CPU rasterizer, `Frame::rasterize` and its primitives (`pixel`, `triangle`, `segment`, `splat`, `ball`, `halo`, `puff`, `disc`, `ray`, glyphs, the sky), and the finished-frame annotations `stroke` and `circle`; the GPU backend mirrors it |
| `render/draw.rs` | The world-space emitters a view records with (`Frame::project`, `facet`, `quad`, `line`, `trace`, `glow`, `beam`, `hairline`, `hairline_between`, `sphere`, `pick_disc`, `outline`, `world_sphere`), and the shared helpers `cover`, `similar`, `glows` and `mass_radius` |
| `render/stage.rs` | `RenderInput`, `Stage` with its `dt` policy (`FrameClock`, `MAX_DT`), `TourTarget`, `Title` and `ring_bounds` |
| `render/scene.rs`, `render/scene/tests.rs` | `Scene` (frame-wide state only, plus the module of each registry row, which it calls through `ViewModule`): smoothing, births and deaths, `fit`, `draw_links`, the exit flashes, `admitted` and `descendants`, and the label names (`label_name`, `without_apple_prefix`); its tests are in `scene/tests.rs` to keep each file short |
| `render/shared.rs` | Constants the CPU rasterizer and `shaders.wgsl` must agree on, declared once; `prelude` writes them into the WGSL source. `LIGHT`, the one world light direction every lit surface shares, is Rust only: shading is baked into colours before the backends see them |
| `render/strokes.rs` | Wide strokes built only from `Item::Triangle`s: `Frame::ribbon` (a screen-facing band along a polyline, `Width` in world units or pixels, an optional `Dash` measured along the 3D arc length) and `Frame::tube` (a flat-shaded prism with parallel-transported `ring_frames`, lit by `shared::LIGHT` through `lit`; `tube_faces` is its pure world-space geometry); every piece is at most 32 px an edge, so a triangle's box is at most 64 x 64 px |
| `render/field.rs` | The field primitive (`Field`, `Placement`, `Relief`, `Shading`, `Frame::field`, `Item::Field`): a grid of colours drawn in world space, flat or as lit relief, sampled bilinearly by the one formula both backends use (clamped uv in node units, `floor(bilerp + 0.5)` per channel, the owner of the nearest node as the pick), with `ShadedField` (the shaded colours and owners a frame keeps and recycles through `Buffers`, so an unchanged `(key, revision)` is not shaded again), the CPU span rasterizer `Frame::field_triangle`, and `StorageLayout`, which plans what the GPU storage buffer must rewrite. Grid simulations draw through it, never one triangle pair per cell; Ripple and Flow still do; Ouija is the first view to draw fields, two flat ones of colour, and the heights, owners, shading and displaced relief have no view yet |
| `render/text.rs` | `Frame::stroke_text`: Hershey text as scene geometry on any world-space plane (`origin`, unit `right` and `up`, `height` in world units, `Align`); weight 0 is one-pixel `Frame::line`s, a weight above 0 one `Frame::ribbon` of `Width::World` per segment, each lengthened by half the weight to close the joins; unpickable (`NONE`), no new `Item`, so the GPU needs nothing; its callers clean process text first (as `terminal::SafeText::new` does) |
| `hershey.rs`, `hershey/romans.jhf`, `hershey/romant.jhf`, `hershey/NOTICE`, `hershey/PROVENANCE` | The Hershey Roman Simplex and Triplex stroke fonts, pure: `parse` reads the `.jhf` records as the Usenet distribution gives them (vertex count, not newline, ends a record; `" R"` lifts the pen; y points down), `Font` (`get` is None for a character outside 32 to 126, `glyph` draws it as `?`, `advance`, `lay_out` into polylines in cap heights with the baseline at 0, `Align`), `font(Face)` (parsed once into a `OnceLock`). The `.jhf` files are unmodified and **must stay so**: the distribution's terms require `NOTICE` to be shipped verbatim with them and forbid converting the data to the NTIS format. `PROVENANCE` has the package, URLs and SHA-256 of each file. Never retype or regenerate glyph data. One record (Triplex `J`) has bounds that miss its own ink, which `decode` repairs in code, not in the file |
| `arbor.rs` | The Arbor module view: the process tree as a tree. The trunk is PID 1 (else the root of the largest subtree, or a pale launchd), each child sits on a phyllotaxis slot (`Slots`) its parent hands out in the order it first sees it from a counter that only rises, and branch radius is `R_TRUNK * sqrt(subtree area / trunk area)` over stretches whose areas `Tree::levels` takes junction by junction (Leonardo's rule). Kernel processes below the trunk grow as roots under a soil slab, which is opaque, so `surface_distance` moves their points toward the viewer until they lie just in front of it. Leaves are threads (at most 32 per twig and 6,000 in all, `leaf_specs`), a `Fall` is the leaf of an exited thread, blossoms follow `Stage::age`, and each branch is a damped `Oscillator` stepped by `FixedStep` in a wind set by CPU pressure |
| `anthill.rs` | The Anthill module view: syscalls as foraging ants. `Anthill` keeps the layout (`Slab`, `Packer`, `Parcels`), the `Terrain` and `Colony`, the `FixedStep` clock and `Scale`, the last `Plan` and what it drew (`Nest`s, pile `Record`s that fade after their file closes, blanket slots, `Conduit`s); `draw` advances the colony to the frame's time, takes in a new sample (`ingest`) and paints (`paint`). `nest_radius` and `pile_radius` are the size mappings, `Thing` kinds `"file"` and `"terminal"` its pickables, and `card` builds a pile's or a blanket's card on demand. A nest is stood for every sampled process, drawn or not, so a limit or a focus moves nothing; its tests build a scene over a row of their own |
| `anthill/flows.rs` | What a sample tells the colony, pure: `Plan::new` (the `Stream`s of ants from nests to `Goal`s, the `Pile`s of the open files with their `Holder`s, the `Tunnel`s between nests, the blanket `Candidate`s and the `Tally` of what could not be drawn). Calls are split over the files in proportion to attributed bytes and never exceed the process's own; a read-write file's direction is `Way::Either`; what no file explains travels the process's tunnels, or goes to a blanket when it has none and holds a terminal, or is counted; macOS (`Unit::Kibibytes`) counts KiB moved with unsized crumbs |
| `anthill/slab.rs` | Where things stand, pure: `Slab` (a clearing and the five-region pinwheel round it, growing in steps of 1.25 only when its nests overflow), `Region::of` (a path's region and second-level directory), `Parcels` (the cell each directory keeps and the spot a path hashes to) and `Packer` (the room each nest reserved, found along a golden-angle spiral with a spatial index, so a thousand nests cost one walk) |
| `anthill/colony.rs` | The colony, pure: `Colony` (ants and `Pheromone`, stepped by `STEP` from `Sending`s), `Ant` and `Route` (steer to a Bezier carrot, smell at plus and minus 0.5 rad, noise of order sqrt(dt), each ant's generator seeded from its nest, goal and sequence), the pheromone grid (three channels, four cells to the unit, at most 256 a side, evaporating as `exp(-dt / 2.2)` then a three-tap blur), the 3,750-ant cap and its refusals |
| `anthill/scale.rs` | K, what one ant stands for: `smallest` (the least of 1, 2, 5, 10, ... that keeps `load / K` under 3,000) and `Scale`, which sets K at once from the first sample with something to carry and then changes it only after three samples in a row agree |
| `anthill/terrain.rs` | The ground, one field: `Terrain` (the static sand with the region tints, string outlines and rim, built once per slab) painted every frame with the pheromone's glow, drawn with `Frame::field`, and the slab's cut sides |
| `anthill/draw.rs` | Drawing the anthill from ordinary items: `mound` (a paraboloid of 24 segments, 12 under 8 px, with a crater and up to four entrances), `pile` (at most 24 pebbles from a path hash, a pick disc), `blanket`, `ants` (two-triangle quads and crumb spheres sized by `crumb_px`), `lane` (a dashed ribbon of a tunnel) and `ball`, a sphere that is not swollen to 3.5 px |
| `anthill/ingest.rs` | Taking in a sample, once per sample and set of drawn processes: the nests, the blanket slots, the pile records, the routes and K, what each nest sends, the entrances and the tunnels' paths |
| `anthill/paint.rs` | One frame of the anthill and the stage's outputs: the order things are recorded in, the tunnels the `c` key shows, the labels (regions, the 12 busiest piles, the biggest and smallest crumb), the camera's bounds (the slab's corners), the tour's stops and the inspector's lines |
| `anthill/words.rs` | What the anthill says, as plain strings for the app to make safe: `bytes`, `count`, the legend (`legend`, with `status::missing_note` for what the platform lacks), the pile card (`pile_card`) and the nest's popup lines |
| `anthill/fixtures.rs` | Test only: the helpers (`process`, `counting`, `holding`, `file`, `snapshot`, `scene`, `play`, `state`) that the tests of the `anthill` files share |
| `cores.rs`, `cells.rs`, `strata.rs`, `globe.rs`, `reef.rs` | Module views, each a state struct that implements `ViewModule` (`draw` and `legend`): the CPU core track, cgroup petri dishes, a minute of CPU history as ridges (Strata keeps only its ridge rows; the profile is read from the shared `History`), TCP connections on a turning earth, and the coral reef. Globe's `face`, `turn` and GeoIP `sources` stay on its row |
| `matrix.rs`, `glyphs.rs` | The Matrix view (`Rain`, drawn from journal lines, not processes) and its 7x14 bitmap font |
| `geo.rs`, `places.rs`, `coastline.bin` | Local GeoIP lookup and home from the time zone (`geo.rs`), country label points and legacy GeoIP country order (`places.rs`), and the packed coastlines the globe draws |
| `lunar.rs` | The moon as an observer sees it, pure functions of the wall clock and a place (the Necropolis moon): `Moon::at` (lit fraction from the elongation of the Astronomical Almanac's low-precision sun and moon series, waxing or waning, and with an observer the `Sky`: topocentric altitude, azimuth, and the bright limb's angle from the zenith, from Meeus's position angle less the parallactic angle), `Moon::phase` (the name) and `wrap_degrees`; tested against the USNO's phases and a table of PyEphem positions |
| `ptolemy.rs` | The Ptolemy module view: isotop as Earth, the seven most varied processes as planets on deferents carrying `simulation::dft` epicycle chains that replay their last minute, trails tinted where retrograde, and every other process a fixed star (`stellar`) with novae and fades from `History::changes` |
| `tesseract.rs` | The Tesseract module view: the `Worldlines` recorder (cumulative CPU time at most every 0.25 s for 180 s, recorded in every view) and its seat bands, the 4D rotation, the Lorentz boost and the display compression, then the Schlegel-projected worldlines, heads, fork vertices and I/O marks |
| `necropolis.rs` | The Necropolis module view: deaths from `Snapshot::life` buried by `record` in every view (rows by UTC minute, families together, mass graves, OOM claims, mausoleums for stopped units, unmarked markers, ghosts on pid reuse, the 600-grave cap), stones built once per grave, and the fogged cemetery, zombies in pits and the real moon drawn from them, in the strip of sky along the top edge where `lunar::Moon` puts it for the snapshot's `home` and `wall` (`moon_over`, `moon_center`, `in_window`, `facing`, `where_is`, `draw_moon`; nothing is drawn while it is below the horizon, and without a home it keeps the top right corner); `lifetime_text` and `Death` |
| `neurons.rs` | The simulation behind Cortex, pure: Izhikevich neurons stepped by 0.5 ms ticks (`Firing`, `advance`, `calibration` and `current` from CPU), a `Network` of neurons, synapses and one event queue of spikes in flight, Golomb's synchrony from exponential averages (`Network::synchrony`), and the `sparkline` encoder for the braille membrane trace |
| `cortex.rs` | The Cortex module view: a neuron per process in a layered slice (seats by kind, widening in 1.25x steps), dendrites per thread, axons from `Snapshot::links` and `pipes` with their delays, the pulses in flight, the flashing somas, the dust of exits, the inspector lines and braille sparkline, and the raster panel drawn in screen space at the bottom of the frame; `Slice`, `Body`, `Axon`, `weight_of_rate` and `raster_bins` |
| `plumbing.rs` | The Plumbing module view (issue 49): a process with a drawn link is a tank (size from memory, band from kind) with a gauge for its attributed outflow; a pipe or FIFO is a copper run with a window, a Unix socket a brass tube overhead holding capsules, loopback TCP a green hose, and a pipe nobody reads any more bursts. `Plumbing` keeps what the view remembers between samples (`take_in`: the pipework of the sample, the readers seen, the `Break`s, the dash phases; `stand`: who is a tank, `LINGER` and `MOST_TANKS`), `Drawn` (what the legend counts and the cards are built from) and the `ViewModule` impl |
| `plumbing/flow.rs` | What a process's flow is, pure: `outflow` turns its unattributed write bytes, its writable files and descriptors and its TCP sends into an `Outflow` (`Measured` with its `Outlets`, or `Unmeasured` with a `Why`), so a lone pipe's flow is exact only when no writable file can hold some of it and several outlets are `split unknown`; `water_speed`, `gauge_fill` |
| `plumbing/room.rs` | `Room::read`: the pipework of one sample as the view draws it, for the processes shown: `PipeRun`s (writer to reader, a FIFO opened both ways running both), `TubeRun`s (Unix socket pairs with a `Lane` of unread bytes each way), `HoseRun`s, the severed pipes, each process's `Outflow`, wait channel and link count |
| `plumbing/floor.rs` | Where the tanks stand, no drawing: `Floor` gives each cgroup a bay (`Bay`, a block of seats placed by `Discs` and snapped to the `CELL` grid, never shrinking) and each tank a seat on a square spiral (`seat_cell`, `PITCH`), and the room that holds every bay so far |
| `plumbing/route.rs` | `Router`: copper runs found by A* on a lattice of 0.5 round `Hull`s (tanks at a fixed routing radius, manifolds), at most `ROUTES_PER_FRAME` new ones a frame, cached until an end moves or a new hull stands on one; cost of a turn 2.5 and of reusing a node 7; `polyline`, `free_sides`, `elbow` (the provisional run) |
| `plumbing/path.rs` | Polyline helpers for pipes in world space: `length`, `at`, `between`, `window` (where a run's glass window sits), `rounded`, `lifted` |
| `plumbing/card.rs` | The inspector cards of what the view draws that is not a process: `Subject` (a run of pipes, a severed pipe, a tube, a hose, a manifold) with the figures a card needs, `card` (built on demand, names raw until the app cleans them) and `flow_lines` |
| `plumbing/paint.rs` | One frame of the view, in stages (`paint_pipes`, `paint_severed`, `paint_tubes`, `paint_hoses`, `paint_manifolds`, `paint_stubs`, `paint_tanks`) and `announce`, which tells the stage the tank places, the room's bounds, the bay titles, the labels, the tour, the popup and callout lines |
| `plumbing/draw.rs`, `plumbing/draw/tank.rs`, `plumbing/draw/pipes.rs`, `plumbing/draw/tubes.rs`, `plumbing/draw/scenery.rs` | Drawing for Plumbing through `Ink`, a wrapper over `Frame` of prisms, pipes, bands, balls and discs built only from triangles and the existing items: the tank and its gauge (`Reading`), the pipe with its window of water (`Water`), the manifold, cap and burst, the tube with its capsules (`capsules`, `CAPSULE_BYTES`, `MOST_CAPSULES`) and the hose with its beads, and the room, bays and walls |
| `plumbing/tests.rs` | Tests of Plumbing as a whole, through `Scene`: bursts, unmeasured flow, the manifolds, the demo's cast, stable layout under `rescaled` and `churned`, the linger and the cap, the tour |
| `horizon.rs` | The Horizon module view: the OOM killer as a Schwarzschild black hole. Processes ride circular orbits in a lensed disk ranked by `oom_score` (rank 1 on the ISCO, `oom_score_adj` -1000 in a band far out; memory footprint where the platform has no score), and a rise of the OOM kill counter (vmstat, else the cgroups' sum) with a process vanishing sends the best-ranked victim on a plunge that freezes short of the horizon; the camera maps to inclination and azimuth, `Roster` and `kill_counter` read the sample, `Horizon::draw` draws back to front by layer, and the demo kill cycle is the row's `DemoOptions::oom_cycle` |
| `horizon/lens.rs` | Pure physics of the Horizon view, no drawing, in G = c = M = 1: `LensTable` (photon rays traced back from the observer by RK4 on d2u/dphi2 = 3u^2 - u, built once on first use, giving the primary and secondary image of a point), `Observer` (images of disk and sky points), `flux` (Page-Thorne), `one_plus_z`, `disk_emission`, `angular_velocity` and `period`, `PlungeTrack` and `plunge_brightness`; each ports a function of `docs/mockups/code/horizon.py` and is tested against analytic values |
| `horizon/orbits.rs` | The Horizon view's population with no drawing: `Ranking` (the swap margin), `Orbits` (bodies eased to the radius of their rank, ghosts of what vanished, the kill counter's rises that turn the best-ranked ghost into a plunge), `Ghost::pose`, `rank_radius` and `band_radius` |
| `horizon/disk.rs` | What the Horizon view draws as lensed triangles: the disk cut into a primary and a secondary image (cells about 4 px, at most 120k triangles, lensed again only when inclination or scale changes, recoloured every frame), `Turbulence`, `SkyField`, the stars, `draw_shadow`, `draw_ring`, `draw_dashed` and `draw_glare`, and the depth `layer`s |
| `switchboard.rs` | The Switchboard module view: interrupts as a cord telephone exchange, Linux only (`available` is false on macOS, so Tab skips it and `--verbose` names it). It reads only `Snapshot::interrupts`, `softirqs` and the effective affinity of B4 (`Source::Interrupts`, `Softirqs`, `IrqAffinity`), so pause and rewind replay history, and draws no process (`draw` returns 0, no tour, the header line is `inspector_line`). A jack is an IRQ line, an operator a CPU, a cord runs to the CPU with the largest per-CPU rate of the interval, the softirq board lamps are per-CPU softirq rates (a rate not measured is a dark lamp, never a quiet one); jacks, lamps and cords pick the line through `Thing` and `ViewModule::card`; `stage.bounds` is the room's corners |
| `switchboard/lines.rs` | What the Switchboard makes of the table, a state machine fed one sample at a time: `device_key` (the queue suffix removed), `Reading` (one snapshot: lines, named rows, softirqs, with None rates kept None), `Line` (`rate`, `servicing`, `shares`), `jacks` (cap 128: lines whose count never changed go first, then devices of more than 8 lines fold into one jack), `Pegs` (seats kept for a line's lifetime, freed ones reused lowest first), `Roster` (P, then E, then unknown cores; above 64 CPUs a seat covers several), `Lines::take_in` and its cords (`Cord`, `Swing`: an affinity change swings at once, a new servicing CPU after two samples in a row, a reseeded board never swings) |
| `switchboard/cord.rs` | The Switchboard's cords, pure: `catenary` (arc length fixed, solved by bisection on the log of the parameter), `slack` (1.02 to 1.07 from the IRQ number), `width`, `operator_color` (warm for P cores, cool for the rest) and `plug_at`, the lift of 0.35 s, the arced smoothstep swing of 1.05 s and the seating of 0.25 s |
| `switchboard/lamp.rs` | The Switchboard's lamps, pure: `brightness` (log10(rate + 1) against 8,000 for a jack, 5,000 for a softirq), `flicker_hz` (the rate capped at 0.9 of `--fps`), `events` (event k at (k + jitter) / f, one to a slot, so stateless and repeatable) and `flash`, `jack`, `fluttering` and `softirq` brightness |
| `switchboard/scenery.rs` | The Switchboard's room and `Pen`: the board's coordinates (the prototype's design pixels, `x` along the board, `d` towards the viewer, `z` up, turned into world units with a built-in shear for the slight yaw), `Plan` (jack places, plug seats, operator seats and the camera-fit corners), the furniture that carries no data (wood, cornice, keyshelf, cabinet, floor) built only from axis-aligned quads, and the pen's primitives (`face`, `disc`, `ball`, `stroke` as a ribbon, `glow` as a puff capped at 12 px at 1080 lines) |
| `switchboard/figures.rs` | The Switchboard's people and what stands on the shelf: `operator` (motion amplitude is the CPU's busy share), `holes`, `telephone` (rattles while RES is at least 1 a second) and `rail` (the clock hand turning once per 6,000 `LOC` counts, the green CAL/TLB lamp and the red NMI lamp lit for 2 s); the figures, chairs and phones carry no data |
| `murmuration.rs` | The Murmuration module view: one bird per thread in a single starling flock (`Flock`: the prototype's alignment over seven topological neighbours with same-process neighbours counting double, cohesion over 21, separation, the roost's pull, the falcon and its agitation wave, stepped at a fixed 20 Hz through `FixedStep` and `SpatialHash3::nearest`), the roster that thins the flock to `BIRD_CAP` (8,000) birds by the lowest hash of tid, birds that rise from and fall to the roost, the falcon released by memory pressure or an OOM kill (`Watch`: a view entered after a gap releases nothing), the flock drawn as stacked `Item::Puff` splats far to near with pick-only discs, and the treeline silhouette, which carries no data |
| `ouija.rs` | The Ouija module view: the newest journal line spelled one letter at a time by the planchette of a talking board by candlelight (`Ouija`: takes each sample's `Snapshot::journal` in once, the first lines after the view is shown being a backlog of which only the newest is queued; `ViewKey::Quit` starts the `Farewell` and `Stage::quit` ends the app when it lands; `tremor` is the jitter of CPU pressure), with the module's own legend and ` > ` status line, no processes drawn and a camera fit by `Stage::bounds` |
| `ouija/seance.rs` | The pure state of Ouija, with no drawing: `Seance` (`push` a `journal::Line`, `advance` by `Stage::dt`; the queue capped at `MOST_QUEUED` with the oldest dropped and counted as skipped, `plan` of an `Entry` into `Stop`s: NO or YES first, then at most `MOST_VISITS` letters, digits and visits to the centre for runs of spaces, `period` of the backlog, `Leg`s that glide by smoothstep with a sideways bow and dwell, the `Reading` and how much of it has decoded, the lit mark, lingering and settling back to the centre), `jitter` (four sinusoids scaled by CPU pressure) and `Farewell` (the glide to GOODBYE, started only by `press`) |
| `ouija/layout.rs` | Where everything stands on the board and the plane it lies on: `Mark`, `spot`, `letters`, `WORDS`, board coordinates in board widths (`Spot`) and `Plane`, which maps them to the world for the default camera, constant whatever the machine does |
| `ouija/scenery.rs` | The wood of the board and the velvet of the table as two `Field`s of baked colour lit by the two `CANDLES` (falloffs computed once, `flicker` recoloured each frame when it moves a step), the value noise they are made from, and the candle light that tones the paint |
| `ouija/paint.rs` | The ink on the board: `Ink` writes the letters, digits, YES, NO and GOODBYE as Hershey strokes (`Frame::stroke_text`) on the board's plane and strokes the double border, the rules, the sun and the moon as ribbons; the lit letter is restroked bright |
| `ouija/props.rs` | What stands on the table: the edge of the board, the two candles (tubes, tongues of flame in screen space, halos as `Item::Puff`) and the planchette (a heart with a hole for its window, a brass rim and ring, a shadow and the glow of the lit letter) |
| `ouija/ribbon.rs` | The decoding line under the board, in screen space with the bitmap font: decoded characters amber, the rest scrambled violet (decorative), the line's source and severity beneath; every character passes `SafeText::new` first |
| `coop/mod.rs` | The Coop module view: `Coop` (the state; `ViewModule` with `record`, `draw`, `legend` and `anchors`), `ingest` (CPU history and OOM kills from each sample), the `Call`, `Departed` and `Fox` records and `summon`, which turns kills into fox runs, `Drawn` (the drawn processes as measured), and what the submodules share (`puffs`, `DUST_RATE`, `FOX_SECONDS`, `plural`) |
| `coop/sim.rs` | The pure yard simulation, the reference shape for a simulation view: `Role`, `Chicken`, `Home` and `Yard` (the Vicsek `step`, `separate`, `confine`, `order`, `flock_orders`), the `YardClock` over `FixedStep`, and `noise`, `speed` and `peer_weight`; no drawing and no process data, each chicken seeded by `Rng::for_identity` |
| `coop/layout.rs` | Where Coop's things stand: `flock_key`, the perch `ladder`s and `seat_offset`, `house_radius`, `Feeder` and `place_feeders`, `Flock` and `Nest` with `nest_seats`, the fence `perimeter`, and `Coop::arrange` (`group_flocks`, `reserve_houses`, `grow_fence`, `lay_feeders`, `found_flocks`: seats, discs, the fence that only grows, feeders and flocks for a sample) |
| `coop/roles.rs` | The per-sample decisions of Coop, in the order `Coop::decide` takes them: `call_roll` (hidden chickens come back, newcomers `hatch`), `judge` (`next_role`, `change_role`, the lock holder and nest, `measure`, `measurement_notes`), `seat_nests`, `describe`, `place_first`, `pull_peers`, `count_unreadable` and `raise_eyes`, with `Pass` and `Roll`, what a pass reads and builds |
| `coop/feeding.rs` | The feeder queues of Coop: `roost_and_gather` (roosters to their seats, running chickens to the feeder of the CPU they ran on), `queue_feeders` and `queue_line` (slots, queue columns, `feeding_note`) |
| `coop/draw.rs` | Drawing Coop's yard: `paint`, the ground, fence, hedge, feeders, henhouses, nests and eggs, and the colours of the props |
| `coop/creatures.rs` | Drawing Coop's creatures: `draw_chicken` (a `Bird` in a `Pose`, drawn shadow, mud, legs, torso and head), `draw_chicks` and `draw_fox` |
| `coop/fixtures.rs` | Test only: the helpers (`process`, `snapshot`, `render`, `coop_of`, `forager`, `scatter`) that the tests of the `coop/` files share |
| `hive.rs` | The Hive module view: memory as honeycomb frames, threads as bees. A hive is a cgroup (an app on macOS), laid out with `layout::Discs`, and shows its three largest processes as frames (`MAX_HIVES` 8, `FRAMES` 3, a shown hive replaced only by one a quarter heavier; hives, discs and slots come from the whole sample, so a focus or `--limit` only hides frames and hives, `Group::admitted`, and clearing it moves nothing). A frame is a fixed 64 x 32 comb drawn as one `render::Field` (`Lattice`: the texel to cell map and the per-look colour tables, built once; `Comb` repaints only when its `Counts`, owner or dimming change). `Measure::of` reads the process's `memory_detail` into a `Mode` (`Split`: honey `Pss_Anon`, pollen `Pss_File`, royal jelly `Pss_Shmem`, capped honey `SwapPss`; `Pss`: no split, kernels before 5.3; `Unsplit`: `Process::memory` as pale nectar, for a rollup still `Pending` or `Source::MemoryDetail` missing; `Sealed`: an `Unreadable` rollup, a wax cap with no cells and no bees), `Counts::of` rounds each kind to cells at the scene's unit (`next_unit`: a power of two from 64 KiB that keeps the largest frame between 700 and 2,000 cells), and the brood is the growth of `Process::memory` since the sample before (`previous_sample`, from the shared `History`), carved out of honey or pale nectar. A bee is a thread (`wishes`, `Swarm`, at most `MAX_BEES` 200 a frame, the leader the queen), stepped at a fixed 20 Hz through `FixedStep` with a speed from `Thread::cpu`; a thread without a CPU reading stands in the idle cluster and is counted in the legend. Nothing is drawn for a process that is in no hive's top three |
| `simulation.rs` | Shared pure simulation helpers: `SpatialHash` (2D) and `SpatialHash3` (3D, with exact k-nearest) neighbour queries, the seeded `Rng` (uniform and `normal`), `Complex` with `dft`, `fft` and `inverse_fft`, the `FixedStep` clock, hashes, and the `growth`, `sprout` and `smoothing` easing |
| `status.rs` | The legend wording for optional sources, so every view says it alike: `missing_note` (` \| no <source>: <consequence>`) and `sources_note` (` \| sources: a, b`) |
| `stellar.rs` | Stellar photometry for views that draw processes as stars: `magnitude` (from memory, `-2.5 log10(memory / 1 GiB)`, floored at 1 MiB), `spread` (core and halo size and strength from a magnitude and the frame height), `Frame::star` and `Frame::star_with` (one pickable pixel-sized sphere and at most one halo, existing items only; `Halo::Glow` for a few stars, `Halo::Puff` for a sky of them, whose halos overlap and must round alike on the CPU and the GPU), and the nova, fade and supernova timelines (`Flash`, `exit_flash`, `flash`), which depend only on age |
| `hr.rs` | The Hr module view: a Hertzsprung-Russell sky of processes. Every process that is not a kernel thread is a star: `temperature` from CPU (2400 K x (1 + cpu / 5)^0.47, at most 40,000 K, drawn through `blackbody::color`), `star_magnitude` from memory (`stellar::magnitude`, 0 B is +9), a spectral class and subclass from the temperature (`spectral_class`) and a luminosity class from its distance below the machine's own main sequence (`luminosity_class`, `main_sequence` by `theil_sen` of magnitude on log10 T), joined as the inspector's `K3 III` by `classification`, and a twinkle from the variation of its CPU over ten seconds (`twinkle_amplitude`). Cgroups are constellations (`layout::Discs` in capacity buckets, a sticky layout scale, boundaries from a fixed grid, stars seated by identity hash and frozen), socket links are stick figures, kernel threads are the Milky Way's dust (`Dome`, never pickable) and the six brightest stars carry class labels. `m` flies every star to its diagram place (`diagram_position`, a pure function of temperature and magnitude; `flight`, 1.5 s from `Stage::time`) and `--hr-diagram` starts there (`ViewOptions`); the mode is module state. The dome is a billboard (`ViewInfo::billboard`) drawn as an opaque `Field` of colours, with `Plot::world` mapping sky units to the ground point that shows at that screen offset from the middle of the screen, less the pan `Hr::follow` keeps from the camera centre's moves, so turning or tilting a panned camera leaves every star on its pixel. `record` brings only the layout and the per-identity records up; `derive` works the fit, the constellation names and the diagram text out when a frame draws, once per sample. A star's class, label and residual come from the newest sample (the same values the fit is made from), its place, colour and size from the eased ones. A nova is a birth and a supernova an exit last seen above 1 GiB (`Record`, which outlives the process for `RECORD_KEEP`; zombies are not stars) |
| `blackbody.rs` | Blackbody colours: `TABLE` (256 linear sRGB entries from 1000 K to 40000 K, geometric in temperature, regenerated by the ignored test `print_blackbody_table`), `linear` (clamped, interpolated in ln T) and `color(kelvin, brightness)` (sRGB-encoded `Color`) |
| `palette.rs` | Every process-kind, process-state and core-kind colour, `mix`, `tint`, `kind_color`, `core_color` and `core_name` |
| `geometry.rs` | `dot`, `normalize`, `normalize_or_zero` and the shaded `cuboid` |
| `keys.rs` | The keymap, the one source of the key bindings: `KEYMAP`, a table of `Binding`s (`Context`, `Group`, `KeyPattern`, `Action`, the README row as `Docs` and the `Hint`s of the help lines); `lookup` (the press's `Action` in a context: the pattern requiring the most modifiers wins, then the table's order, and a `Text` pattern only for a character nothing else binds), `line` (a help line from the hints, which `App::text` shows), `tables` and `markdown` (what `--keys` prints and the README's `## Controls` tables hold, checked by `readme_controls_match_the_keymap`), `summary` (the end of `--help`) and `LOCAL_KEYS` (the characters a view may claim) |
| `picker.rs` | The view picker opened by `v` and the Tab playlist: `Picker` (filter, highlight, popup lines), the name and title filter, `playlist` (the `--views` rules) and the `cycle` and `toggle` rules; pure, so the tests drive it without a terminal |
| `layout.rs` | Stable placement: `Seats`, `Discs`, the golden-angle `spiral`, `vacant` and `spin` |
| `lights.rs` | The Light List: stable headlands and lighthouse seats, time characteristics from state, CPU and threads, pure waveforms, memory-derived candela and Allard ranges, fog from I/O pressure, sample-diff notices and extinguishing lights |
| `history.rs` | The shared per-process CPU and memory history: `History` (a ring of `CAPACITY` samples sorted by `Identity`, recorded by `Scene` on every sample whichever view is shown; `window` by `elapsed` time, `recent` and `series` by count, `changes` for the births and exits between consecutive samples), `Reading`, `Sample` (with an absolute `sequence`), `Change`. Views read it through `Stage::history`; `warm_up` replays it for headless demo runs |
| `medium.rs` | Pure simulation state for the Ripple (wave equation) and Flow (particles) views |
| `gpu.rs`, `shaders.wgsl` | wgpu backend that mirrors the CPU rasterizer; the field pass samples one storage buffer by hand (no hardware sampler) and `store_fields` rewrites only the slots whose field was shaded again |
| `terminal.rs` | Graphics and Kitty keyboard capability probe (the keyboard flags are pushed when the terminal answers, and `restore_screen` pops them before the alternate screen is left), frame transfer, text overlay (`Label`, `Popup` and the status rows, all `SafeText`), terminal restoration |
| `terminal/text.rs` | `SafeText`, the only text the terminal overlay takes (`new` and `braille` clean, `trusted` takes a literal, `width` and `cut` count cells), and the cell widths: `cell_width` and `display_width` over the zero-width and double-width ranges |
| `platform/macos/mod.rs` | macOS `Sampler` (libproc, Mach, sysctl, IORegistry, `proc_pid_rusage` V6 with fallbacks), the cluster clocks, the socket scan, the bounded open-file scan (`FileScan`, one `proc_pidfdinfo` per vnode), the pipe and FIFO scan (`PipeScan`: `PROC_PIDFDPIPEINFO`, and a vnode call per vnode descriptor to find FIFOs), the thread reader (`Threads`: `PROC_PIDLISTTHREADIDS`, then `PROC_PIDTHREADID64INFO` per thread), the descriptor reader behind `Source::IoDetail` (`Descriptors`: the listing tells pipes, sockets and kernel objects, and one `PROC_PIDFDVNODEPATHINFO` call per vnode gives its path, `fi_offset` and `fi_openflags`; no byte counters, so `counts_bytes` is false; the pid's start time is checked afterwards), `oom_scores` (root only: `memorystatus_control`), and what it reports through `missing`, `per_cluster` and `unreadable` |
| `platform/macos/ffi.rs` | Every extern declaration and `#[repr(C)]` struct that `libc` lacks, with size assertions (`vnode_fdinfowithpath`, the vnode record with its path, among them); `memorystatus_control` and the jetsam priority entry |
| `platform/macos/logic.rs` | Pure macOS decisions with no FFI: jetsam priority lists (`jetsam_priorities`) and their rank (`jetsam_oom`), kinds, groups, parents, the Mach scheduling policy, `KERN_PROCARGS2` parsing, tick conversion, cluster clocks from cycle counters, socket pairing and queues, open and deleted-but-open files and pipe ends from `vinfo_stat` and `pipe_info`, thread run states and ids, the I/O detail's descriptor kinds (`fd_kind`), vnodes (`vnode`: a regular file with access mode from `FREAD` and `FWRITE`, position from `fi_offset`, `FAPPEND` and a link count of 0, a terminal under `/dev/tty*`, or other) and its ranking by disk I/O rate (`by_disk_rate`) |
| `platform/macos/journal.rs` | The unified-log follower (`log show`, then `log stream`) and its std-only JSON line parser |
| `platform/macos/events.rs` | The `isotop-events` thread on macOS: exec and exit of the sampled processes from kqueue (`EVFILT_PROC`) |

`agents_file_table_lists_every_source_file` fails when a `.rs` file under `src/` has no row. Name
files by path relative to `src/` in the first column; a row may list several.

## Architecture

Dependencies mostly run one way: `platform` and `model` (the data), then the shared helpers, then
`render` with `views` and the view modules, then `gpu` and `terminal`, then `app`, then `cli`, `headless` and
`interactive`, then `main`. Check with
`grep -n 'use crate::' src/*.rs` before adding an import. Nothing imports `main.rs`, `model` and
`platform` never import `render`, a view or `terminal`, and `gpu` and `terminal` import only
`render`. The exceptions, so don't assume otherwise:

- `model` and `platform` import each other: the platform builds `model` types and `model` calls the
  `Sampler`. `journal` and `platform/*/journal.rs` do the same,
  and `model` imports `journal::Delivery` for `Snapshot.journal`.
- `render` imports no module view's state or drawing, only the registry (`views`: `REGISTRY`,
  `ViewInfo`, `ViewModule`), the `Orbit` ellipse that `Frame::trace` strokes (`views::systems`),
  `globe::drift` for the camera's turn of the earth, the `glyphs` bitmap font and the `hershey` stroke font (`render/text.rs`); each view imports
  `render` for `Frame`, `Stage` and `Color`. `render` and `views` import each other: `Scene` draws the rows, and the rows name
  `Scene`, `Stage` and `Backdrop`. `palette`, `geometry` and `medium` import `render` too, for `Color`, `Point` and
  `Frame`. `simulation` and `layout` import only `model::Identity`.
- The test-only `harness` imports `App`, `Options`, `demo_snapshot` and `warm_up` from the crate root,
  which re-exports them from `app`, `cli`, `main` and `headless`, so it runs a view exactly as a headless
  `--output` run does.
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
rule is `DrawnByView`), time, `fps` (the target frame rate, `--fps`, which `Scene.fps` holds and the app sets once; a view that animates faster than a frame shows caps it there), the `frames` the scene has rendered (a view that must tell the next
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
`stage.tags` (`Tag { at, side, text }`: a short text written beside a point, on its `side` if that fits, before `places` and `titles` are offered their cells),
`stage.notes` (popup lines after the sockets), `stage.sparklines` (generated
braille lines right after them, made `SafeText::braille` by the app, never process text),
`stage.tour` (`TourTarget { id,
position, weight }`, which the tour visits heaviest first) `stage.popup` (popup lines right
after the process's own figures) and `stage.callout` (lines for the tour's callout of a process) and `stage.quit` (set once a farewell the view deferred a quit for is over; see `ViewKey::Quit`), and returns how many processes it showed; the rest count as
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
- **Process text is untrusted.** Names, command lines, thread and cgroup names, journal lines,
  socket peers and file paths are stored raw in the model and reach the terminal only as
  `terminal::SafeText`: its field is private to `terminal/text.rs`, `SafeText::new` replaces
  everything but printable ASCII with `?`, `braille` does the same for generated sparklines and
  `trusted` takes only a `'static` literal. `Label::text`, `Popup::lines` and the status rows
  of `Terminal::present` are `SafeText`, so text that skipped the sanitizer does not compile
  (`process_text_cannot_inject_terminal_controls`). A view hands `Stage` plain strings and the
  app wraps each in `SafeText::new` where it builds a label, a popup or a status row; lay text
  out by `SafeText::width` (cells, never `len`) and cut it with `SafeText::cut`. Allowing
  printable Unicode is a separate decision: it needs the width tables checked in Ghostty and
  Kitty, and changes only the constructors.
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
    high the scene lifts its links above the drawn positions, 1.5 over City's rooftops), `flash` and `haze` (on), `pitch` (None: leave the camera's), `billboard` (off: a plane that faces the viewer whatever the rotation and pitch, so the fit measures its `Stage::bounds` under the camera that drew them and not the goal's), `tour` (on),
    `header` (None: the scene's visible and collapsed counts), `face` (None), `turn` (None: the
    wheel pans), `keys` (none: the view's own keys, which lead the `?` help line while it is
    shown), `sources` (none), `demo` (`DemoOptions::NONE`; a row names only the options it turns on, `DemoOptions { churn: true, ..DemoOptions::NONE }`, so a new option leaves the other rows alone) and `contract`
    (`Contract::DEFAULT`: it draws processes, which may move at most 1 world unit over the
    ground, checked where they are drawn at 12 s of demo time, and it is known to fail nothing).
    Tab, Shift+Tab, the `v` picker, `--view`, `--views`, `--list-views` and `--verbose` follow
    from the table. Never reorder the first ten rows: the digits pick them.
  - View-local keys reach `ViewModule::key` as a `ViewKey`: `Local` for `keys::LOCAL_KEYS` (`b i j k m o p u x y z , . ;`) only,
    `Reset` for Home and `r` (the global reset happens as well), `Escape` (returning true
    consumes that press) and `Quit` for `q` (returning true defers the quit until the view sets
    `stage.quit`). Every other key is global, and no key reaches the view while a search or the
    picker is open. A key binding is a row of `keys::KEYMAP`, never a key literal in `App::command`;
    a view's own key is listed there too, as an `Action::ViewLocal` row for the README.
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
