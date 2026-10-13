# Theory: Sampling on macOS

macOS has no `/proc`. Everything isotop knows about a process there comes from `libproc` (`proc_pidinfo`,
`proc_pid_rusage`, `proc_pidfdinfo`), from `sysctl`, from Mach host calls and the IORegistry, and from `kqueue`. The
shape of the data is the same as on [Linux](Theory-Sampling-Linux): cumulative counters that
[the model](Theory-Model-and-Rates) differences, and gauges read as they are. What differs is the units (Mach
absolute time instead of ticks), the identity of a process, what the kernel will not tell an unprivileged user, and
a short list of readings that do not exist. This page follows the code in
[`platform/macos`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/mod.rs) and says where the
README's "What macOS does not report" table agrees with it (it does, row by row; checked below).

macOS code cannot run on Linux, so the pure decisions live in
[`logic.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/logic.rs), which has no FFI and is
also compiled into Linux test builds; the project's CI runs the rest on `macos-15`. Nothing on this page was run
on a Mac while writing it; claims are from the source and its tests.

## History

*Mach.* The kernel under macOS descends from Mach, begun at Carnegie Mellon University in 1985 by Richard Rashid and Avie Tevanian, and developed there until Mach 3.0 in 1994. Apple's kernel, XNU, combines a Mach component with a BSD layer. That is why this page has two vocabularies: Mach's tasks, threads, absolute time and host calls, and BSD's pids, `sysctl` and `kqueue`. Tevanian later went to NeXT and then to Apple.

*libproc.* The `libproc` calls the sampler is built on, `proc_listpids` and `proc_pidinfo`, are marked in Apple's headers as available from Mac OS X 10.5; `proc_listallpids` is marked from 10.7. The header describes them as private interfaces that are subject to change in future releases, and isotop relies on them (and on a private responsibility call, section 2) all the same. I did not find a source that dates `proc_pid_rusage`; the version flavours V2 to V6 in section 1 are the record of it growing.

*kqueue.* `kqueue` was written by Jonathan Lemon for FreeBSD 4.1 (July 2000) and described in "Kqueue: A generic and scalable event notification facility" at the FREENIX track of the 2001 USENIX Annual Technical Conference in Boston. It arrived to replace `select` and `poll`, which scale poorly with the number of descriptors and cannot watch signals or file system changes; `EVFILT_PROC`, the filter this page uses, is one of its extra event sources. Lemon's slides record that the paper was left out of the proceedings and handed out on paper. NetBSD, OpenBSD, DragonFly BSD and macOS adopted it later.

## 1. The platform's side

General macOS background, independent of isotop.

**Mach absolute time.** Mach counts time in hardware-dependent *absolute time units*. `mach_timebase_info` gives a
fraction `numer/denom` that converts units to nanoseconds. On Apple silicon the counter runs at 24 MHz, so the
fraction is 125/3 (about 41.67 ns per unit); on Intel it is 1/1. `proc_pid_rusage` reports CPU times in these
units, so isotop converts them with a 128-bit product (`logic::nanoseconds`) and then treats **nanoseconds as its
tick**: `Sampler::hz()` returns `1e9`. A CPU percentage is then the same expression as on Linux,
$100\,\Delta\text{ticks}/(\text{hz}\,\Delta t)$, with a much finer quantum.

**`libproc` flavours.** `proc_pidinfo(pid, flavor, ...)` returns one of several fixed-layout structures. The ones
used: `PROC_PIDT_BSDINFOWITHUNIQID` (BSD info plus a unique id and a *pid version*), `PROC_PIDTASKINFO` (Mach task
totals: resident size, thread count, priority, page-ins), `PROC_PIDT_SHORTBSDINFO` (pid, parent and name only),
and `PROC_PIDLISTFDS` for the descriptor table. `proc_pid_rusage(pid, flavor, ...)` returns `rusage_info_vN`,
cumulative resource usage whose layout grows with N: V2 adds the physical footprint and disk I/O, V4 adds
instruction and cycle counts and the **runnable time**, V6 splits CPU time and cycles between **performance and
efficiency cores**. Every older flavour is a prefix of V6 (asserted in `ffi.rs`), so one buffer serves all.

**Permissions.** `proc_pidinfo` answers for your own processes and, as root, for everyone's. For another user's
process it fails with `EPERM`, except `PROC_PIDT_SHORTBSDINFO`, which xnu answers without the same-user check
(`proc_pidinfo` in `bsd/kern/proc_info.c`).

**Apple-silicon clusters.** The chips have two kinds of CPU cores, performance (P) and efficiency (E), in clusters
whose clocks are set independently.

## 2. What the sampler reads, per process

`Sampler::processes` lists every pid (`proc_listallpids`, retried with a larger buffer if it filled), then, per
pid:

| call | gives | becomes |
|---|---|---|
| `PROC_PIDT_BSDINFOWITHUNIQID` (argument 1, so zombies are found) | name, parent, status, uid, nice, start time, `p_idversion` | `name`, `parent`, `nice`, `Identity::start`, `started`, `RawProcess::generation` |
| `PROC_PIDTASKINFO` (skipped for a zombie, which has no task) | resident size, total user and system time, threads, running threads, priority, policy, page-ins | `threads`, `state`, `priority`, `policy`, `major_faults` |
| `proc_pid_rusage`, newest flavour the kernel accepts | footprint, user and system time, disk bytes, cycles, runnable time, performance-core time and cycles | `memory`, `ticks`, `system_ticks`, `io`, `runnable_ticks`, `performance_ticks` |
| `proc_pidpath` and `sysctl(KERN_PROCARGS2)`, only when a description is stale | executable path, argument vector | `command`, `group`, `kind` |
| the *responsibility* SPI | the pid of the app responsible for the process | `parent` (see below) |

**Identity.** `Identity { pid, start }` with `start` the process's start time in microseconds since the epoch
(`pbi_start_tvsec · 10⁶ + pbi_start_tvusec`). A recycled pid has a different start time. Separately,
`p_idversion`, the *pid version*, changes on every `exec` even when the pid, the start time and the name do not;
isotop carries it as `RawProcess::generation`, so [exec detection](Theory-Life-Events) on macOS is exact between
samples instead of a guess from the name.

**Memory is the physical footprint** (`ri_phys_footprint` from `proc_pid_rusage`), the figure Activity Monitor
shows. It counts compressed and swapped dirty memory and leaves out clean shared pages, so it is not a resident
size and not comparable to Linux's RSS; `MEMORY_LABEL` is `"memory footprint"`. If `proc_pid_rusage` is refused,
`pti_resident_size` stands in.

**CPU time.** From the same `rusage` reading when there is one, `ri_user_time + ri_system_time`, converted to
nanoseconds, so the performance-core and runnable totals below belong to the same instant; otherwise from
`PROC_PIDTASKINFO` (`pti_total_user + pti_total_system`). `system_ticks` is the system part.

**State, priority, policy.** `state` is `Z` for a zombie (`pbi_status == SZOMB`, 5), `T` when stopped, `R` when the
task has a running thread, else `S`; macOS has no uninterruptible sleep, so never `D`. Priority is mapped onto
Linux's scale as `51 − Mach priority`: an ordinary app at 31 reads 20, background work at 4 reads 47, and
real-time threads above 51 read negative, as Linux's real-time tasks do. `pti_policy` 1, 2 and 4 become normal,
round-robin and FIFO; anything else is unknown.

**Command line.** `KERN_PROCARGS2` returns a native-endian `int argc`, the NUL-terminated executable path, NUL
padding, then `argc` NUL-terminated arguments and the environment. `parse_procargs` skips the padding and stops
after `argc` arguments: the environment sits in the same buffer but is never parsed or kept. A
description is re-read only when `(pid version, name)` changes (`same_program`): exec gives a new version even if
the name stays (a shell running `exec zsh`).

**Kinds and groups.** pid 0 is `kernel_task`, kind `Kernel`. A process whose executable file name or `name` matches a
virtualization helper (Virtualization.framework, QEMU, `limactl`, `vfkit`, `krunkit`, OrbStack, Docker) is a
`Container`. Otherwise it is `System` if its uid is below 500 or its path is under `/System`, `/usr` (but not
`/usr/local`), `/bin`, `/sbin` or `/Library/Apple`; otherwise `Session` if it runs under the session's uid
(`SUDO_UID` when root started isotop through sudo) and `System` if not. The group, which stands in for a cgroup, is the
outermost `.app` bundle in the path or else the executable's file name, and the `cgroup` string is a synthetic
`/<kind>/<group>` key.

**Parents are the responsible app.** macOS reports, through a private function isotop looks up with `dlsym`
(`responsibility_get_pid_responsible_for_pid`), which app a helper belongs to. `logic::parents` re-parents each
process under its responsible app, so launchd's helpers sit under their app instead of under launchd, with three
guards: a process keeps its real parent if that parent already descends from the responsible app (a command stays
under its shell), a choice that would close a loop falls back to the real parent, and processes are settled
shallowest first so a shell is placed before the commands under it. If the function is missing, the real parent is
used.

## 3. System-wide readings

* **Per-CPU load**: `host_processor_info(PROCESSOR_CPU_LOAD_INFO)` returns cumulative scheduler ticks
  `[user, system, idle, nice]` per CPU as 32-bit counters, which wrap. `logic::busy` takes differences modulo
  2³², so a wrap between samples is harmless, and `busy = (user + system + nice) / total`. `None` when no ticks
  passed.
* **Memory**: `hw.memsize` for the total (`sysctl`); available is `(free + inactive) · page_size` from
  `host_statistics64(HOST_VM_INFO64)`. xnu's `free_count` already includes speculative pages, so they are not added
  again.
* **Pressure**: only the memory pressure *level*, `kern.memorystatus_vm_pressure_level` (1 normal, 2 warning,
  4 critical), is available. It is mapped to `0`, `25` and `75` in the memory slot of `Snapshot::pressure`. That is
  a level, not a stall percentage, and the views that tint by memory pressure show a level. CPU and I/O pressure
  have no counterpart at all.
* **Core kinds**: the IORegistry plane `IODeviceTree:/cpus`; each child's `cluster-type` byte (`E`; `P` or `M`, the
  newest chips' middle tier, both counted as performance) and `logical-cpu-id` property give each CPU's kind. Any
  CPU the registry does not cover stays `Unknown`, and the order of ids is not assumed.

## 4. Clusters, cycles and the clock

macOS reports no per-CPU clock. It does report, per process, cumulative **cycles** and CPU time, each both in total
and on performance cores only (`ri_cycles`, `ri_pcycles`; `ri_user_ptime + ri_system_ptime`). Cycles per second of
CPU time is a frequency, so the *average clock a cluster ran at* is recoverable as a ratio of sums over every
process isotop can read:

$$
f_{\mathrm{P}} \;=\; \frac{\sum_i \Delta\text{pcycles}_i}{\sum_i \Delta t^{\mathrm{P}}_i},
\qquad
f_{\mathrm{E}} \;=\; \frac{\sum_i \left(\Delta\text{cycles}_i-\Delta\text{pcycles}_i\right)}{\sum_i \left(\Delta t_i-\Delta t^{\mathrm{P}}_i\right)},
$$

with $\Delta t_i$ the process's CPU time over the interval and $\Delta t^{\mathrm P}_i$ the part on performance cores. The
efficiency cluster is the remainder. A worked case from the tests (`cluster_clocks_divide_each_clusters_cycles_by_its_cpu_time`):
one process ran 300 ms on P at 3.2 GHz and another 100 ms on P at 4.0 GHz, so $(960+400)\,\mathrm{Mcycles}/0.4\,\mathrm s = 3400$ MHz.

This is an *effective* clock, weighted by who ran, not a sample of a register. Guards in `cluster_clocks`: a
cluster with under `CLOCK_FLOOR = 0.05` s of CPU time in the interval has no reading (`None`); a process whose
counters went backwards (a reused pid) or whose parts exceed its whole is skipped; and virtual machines count CPU
time but no cycles, in which case there is no reading and `Source::CpuClock` is missing. Without root, only your
own processes contribute, so the cluster's clock is *your* work's clock. Every `Cpu` of kind P shows the P clock and
every E the E clock, so the legend says "cluster clock" (`Sampler::per_cluster`).

Two more per-process quantities come from the same counters, both handled in the model:

* **`performance_share`** is $\Delta t^{\mathrm P}/\Delta t$, the fraction of the interval's CPU time that ran on
  performance cores, replacing the per-process "last CPU" that macOS cannot give.
* **`waiting`** comes from `ri_runnable_time` (V4 and later), which counts a thread's time while runnable *or*
  running; minus CPU time it is time spent waiting for a core. The kernel may update runnable time only when a
  thread is switched on or blocks, so it can lag CPU time while threads run and catch up later; the collector counts
  only growth beyond the highest `runnable − cpu` ever seen and caps the result at the thread count
  ([details](Theory-Model-and-Rates#counter-resets)).

## 5. Privilege: unreadable processes and shadows

Without root, `proc_pidinfo` fails with `EPERM` for other users' processes. `Sampler::processes` counts them
(`unreadable`, shown as `+N unreadable: run with sudo`) and keeps what anyone may read, their pid, parent and name
(`PROC_PIDT_SHORTBSDINFO`), as `Shadow`s, so that what descends from an unreadable parent (launchd, without
sudo) can still be drawn under a stand-in. Their CPU and memory are never guessed. Sockets and open files follow the
same rule, so every network or file figure on macOS is a lower bound without root.

## 6. Sockets and open files

Both use `proc_pidinfo(PROC_PIDLISTFDS)` to list a process's descriptors and `proc_pidfdinfo` for one descriptor.

* **Sockets.** For each socket descriptor, `PROC_PIDFDSOCKETINFO` returns a `socket_fdinfo` with the protocol union.
  TCP gives local and foreign address, port and state; Unix gives the socket's protocol control block address and its
  peer's. The struct is accepted only if the kernel wrote exactly the size isotop declared (the layouts agree).
  `logic::pair_tcp` pairs established connections whose local and foreign addresses mirror each other; a connection
  to a non-loopback, non-own address is an *outside* connection. `pair_unix` pairs control blocks.
  macOS reports no per-socket byte counters and no round-trip time: `Remote::rtt`, `sent` and `received` are filled
  with `0`, and `Network::loopback` is empty, so `Source::SocketTraffic` is reported missing and the views must not
  read those zeros as measurements. This is the one place where macOS data is a placeholder zero rather than a
  `None`; the contract is that `missing` outranks the value.
* **Open files.** Only vnode descriptors can be files, so only they are examined (`PROC_PIDFDVNODEINFO`, which stats
  the file through the descriptor). A regular file with link count 0 is deleted-but-open and its size is recorded.
  The bounds are Linux's (4096 examined, 16 384 listed per process, 65 536 per scan), the open count past the bound
  is estimated by proportion, a vnode that could not be read marks the table partial, and a zombie holds no files.
  macOS has no `AT_STATX_DONT_SYNC`, so a network mount that has stopped answering can stall the scan once its cached
  attributes go stale.

## 7. What macOS does not report

`Sampler::missing` (via `logic::missing`) returns these, and each is checked against the README's table:

| `Source` missing on macOS | consequence in the model | in the README |
|---|---|---|
| `CpuPressure`, `IoPressure` | zero; status line prints `n/a` | yes |
| `MemoryPressure` | only if the level sysctl fails; otherwise a 0/25/75 level | yes |
| `LastCpu` | `Process::core` is `None` | yes |
| `RunQueue` | `Cpu::wait` is `None`; `Process::waiting` stands in | yes |
| `Cgroups` | no `Unit`s: no limits, quotas, throttling, OOM counts | yes |
| `SocketTraffic` | rtt and byte counters are placeholders | yes |
| `FileLocks` | `locks()` returns `None`: `fcntl(F_GETLK)` only tests a range of a file the caller has open | yes |
| `GpuMemory` | `Gpu::load()` returns `None`; Apple GPUs share memory with the CPU | yes |
| `SyscallIo` | `syscalls` is `None`; no per-process read and write call counts | yes |
| `CpuClock` | missing unless clocked per cluster (V6, both cluster kinds named, cycles counted) | yes |
| `CoreKinds` | only if the IORegistry names none | yes |
| `MemoryDetail` | `Process::memory_detail` is `None`: no `smaps_rollup` | not checked against the README |
| `Interrupts`, `Softirqs`, `IrqAffinity`, `OomKills` | the kernel counters of `Snapshot` stay empty or `None` | not checked against the README |

Four readings that only some views demand also exist here. `Source::Oom` is read from the jetsam priority list (`memorystatus_control`, `MEMORYSTATUS_CMD_GET_PRIORITY_LIST`), which only root may list, so for anyone else it is missing; the rank is the negated band, with no adjustment value (`jetsam_oom`). Unix sockets carry a receive queue (`logic::unix_queue`: bytes for a stream socket, a flag for a datagram socket, none for a listener). Pipes and FIFOs come from `PROC_PIDFDPIPEINFO`, plus a vnode call per vnode descriptor to find FIFOs. Threads come from `PROC_PIDLISTTHREADIDS` and then `PROC_PIDTHREADID64INFO` per thread, with CPU in nanoseconds like the process's.

Not a gap but not the same quantity: `major_faults` is the page-in count (`pti_pageins`, which includes read-ahead
and wraps as an `i32`, so it is widened through `u32`), not Linux's `majflt`. The journal is the unified log
([Journal](Theory-Journal)). Under demanded life events, `Source::ForkEvents` and `Source::ExitRecords` are missing
because there is no proc connector and no taskstats: see [Life events](Theory-Life-Events#2-macos-kqueue).

How a view degrades is the view's business and is described on its page; the model's part is only that a value that
could not be measured is absent, not zero, and that `Snapshot::missing` says why.

## 8. What the numbers do not say

* **Footprint is not RSS.** A figure can be larger or smaller than a Linux user would expect for the same program.
* **The cluster clock is an average over the work done**, not a hardware reading, and a quiet cluster has none.
* **Parents are re-parented by an undocumented SPI.** `Process::parent` is "the responsible app", not the POSIX
  parent, wherever that differs.
* **Without root the picture is partial by construction**, and says so (the unreadable count and the hollow stars).
* **Quantum.** CPU time has nanosecond units, but the kernel's accounting resolution is whatever it is; isotop does
  not measure it, so the Linux "one tick" floor has no number here.

## Where in the code

* [`platform/macos/mod.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/mod.rs): `Sampler`,
  `process`, `describe`, `cpu_ticks`, `core_kinds`, `rusage_version`, `shadow`, `FileScan`, `network`, `Gpu`.
* [`platform/macos/logic.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/logic.rs): `nanoseconds`,
  `busy`, `pressure`, `missing`, `cluster_clocks`, `parents`, `classify`, `group`, `parse_procargs`, `pair_tcp`,
  `descriptor`, `table`.
* [`platform/macos/ffi.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/ffi.rs): every extern
  declaration and struct layout `libc` lacks, with size assertions.
* [`platform/macos/journal.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/journal.rs): the unified-log follower.
* [`platform/macos/events.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/events.rs): kqueue.

## References

* Apple's system headers `libproc.h`, `sys/proc_info.h`, `sys/resource.h` (`rusage_info_v2` to `v6`),
  `mach/mach_time.h`, `mach/host_info.h` and `mach/processor_info.h`.
* The xnu kernel source, as the code cites it: `bsd/kern/proc_info.c` (`proc_pidinfo` permission checks),
  `bsd/sys/proc.h` (`SZOMB`, `SSTOP`), `bsd/kern/kern_sig.c` (`killpg` errors).
* `sysctl(3)` and `kqueue(2)` in the macOS manual.
