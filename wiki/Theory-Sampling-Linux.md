# Theory: Sampling on Linux

On Linux, everything isotop knows about a process is a number the kernel chose to print into a text file, a
cumulative counter that isotop subtracts from its previous value, or a fact it extracted from a netlink
conversation. There is no instrument inside the process and no tracing. This page says what each file contains,
what the kernel means by it, how [`platform/linux`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/mod.rs)
parses it, and what the sampler reports when a file is absent. What happens to the numbers afterwards (rates,
smoothing, aliasing) is on [Model and rates](Theory-Model-and-Rates); how the slow sources are scheduled is on
[the background sampler](Theory-Background-Sampler-and-Demand); the macOS counterpart is
[Sampling on macOS](Theory-Sampling-macOS).

## 1. The kernel's side of the contract

This section is general Linux, independent of isotop.

### Counters, not gauges

Most of the kernel's per-process accounting is a set of **monotone cumulative counters**: CPU time consumed,
bytes read, pages faulted. A counter is the integral of a rate. Read it at two instants and subtract, and you
recover the exact average rate over the interval *whatever the interval is*. Sampling coarsely loses time
structure inside the interval and loses nothing about the total: a counter read once a day still knows how much
CPU the process used that day. A **gauge** (resident memory, thread count, run state) has no such property;
whatever happened between two reads is gone. isotop takes counters wherever the kernel offers them and treats
gauges as what they are, instantaneous. The consequences for estimating rates are worked out in
[Model and rates](Theory-Model-and-Rates#quantisation-and-aliasing).

### Jiffies, `HZ` and `USER_HZ`

The kernel counts time in **jiffies**, ticks of a timer interrupt at `HZ` per second, a compile-time choice
(`CONFIG_HZ`, commonly 100, 250, 300 or 1000). Userspace is never shown jiffies. Every time exported through
`/proc` is converted to **`USER_HZ`**, which is 100 on the mainstream architectures, and `sysconf(_SC_CLK_TCK)`
returns it (see `time(7)`). One *clock tick* is therefore 10 ms. isotop reads it once, in `Sampler::new`, as
`hz`, and uses it as the unit of `RawProcess::ticks`:

$$
\text{cpu\_time}\;[\mathrm{s}] \;=\; \frac{\text{ticks}}{\text{hz}},\qquad
\Delta\text{cpu}\;[\%\text{ of one core}] \;=\; 100\,\frac{\Delta\text{ticks}}{\text{hz}\cdot\Delta t}.
$$

The 10 ms quantum is the resolution floor of every CPU figure on the screen. At the default one-second sampling
interval it is one percentage point of one core; at `--sample-ms 100` it is ten.

### `utime`, `stime`, and what "CPU time" is

`/proc/<pid>/stat` fields 14 and 15 are the CPU time the process has spent in user and kernel mode, in ticks.
For the process-level file (`/proc/<pid>/stat`, as opposed to a thread's `/proc/<pid>/task/<tid>/stat`) the kernel
reports the **sum over every thread of the process, including threads that have already exited**. isotop's
`ticks` is `utime + stime`; `system_ticks` is `stime`.

The *total* is the scheduler's own runtime accounting. The *split* is not: on the usual tick-based accounting
configurations the kernel estimates which mode a task was in from where the timer tick found it, and scales that
ratio to the exact total (`cputime_adjust` in `kernel/sched/cputime.c`; I have not re-checked this against a
kernel tree for this page). So `ticks` is trustworthy to a tick, and `system_share` (the kernel-mode fraction)
is a statistical estimate that is noisy over short intervals.

### Resident set size and the page size

Field 24, `rss`, counts the pages of the process that are resident in RAM: anonymous, file-backed and shared
memory alike. A shared page is counted in the RSS of *every* process that maps it, so the RSS column of all
processes sums to more than the memory in use. It is a count of pages; isotop multiplies by
`sysconf(_SC_PAGESIZE)` (4 KiB on x86-64, 16 or 64 KiB on some arm64 kernels). `Process::memory` is this number
and `MEMORY_LABEL` is `"RSS"`. The field is parsed as a signed integer and clamped at zero
(`stat_handles_spaces_parentheses_and_negative_rss` exercises a `-1`).

### Names are not identities

A pid names a process only while it lives; the kernel recycles pids. `/proc/<pid>/stat` field 22, `starttime`,
is when the process began, in ticks after boot. The pair `(pid, starttime)` is isotop's
[`Identity`](Theory-Model-and-Rates#the-data) and is what every table in the program is keyed by. Two processes
share an `Identity` only if one pid is reissued within the same 10 ms tick, which would need a complete wrap of
the pid space in that time (more than three million forks a second at the common `pid_max` of 32768). That is an
argument, not a measurement, but it is a very large margin.

## 2. What the sampler reads on every sample

`Sampler::processes` runs on the frame loop, once per `--sample-ms`. It lists `/proc`, keeps the entries whose
names parse as a `u32`, and for each one reads `stat` and `io`. That is two small files per process and no more
in the steady state; the command line and cgroup come from a cache (below).

### `/proc/<pid>/stat`

The line is `pid (comm) state ppid ...`. The command name `comm` can contain spaces and parentheses, and the
kernel does not escape them, so `parse_stat` takes the text between the first `(` and the **last** `)` as the name
and splits the remainder on whitespace. Indices below are into that remainder, which starts at proc(5) field 3.

| proc(5) field | name | index | becomes |
|---|---|---|---|
| 3 | `state` | 0 | `Process::state`, the first character (`R S D Z T t X I`) |
| 4 | `ppid` | 1 | `Process::parent` |
| 9 | `flags` | 6 | bit `PF_KTHREAD` (`0x00200000`): `Kind::Kernel`, and the file table is known to be empty |
| 12 | `majflt` | 9 | cumulative major page faults, rated by `measure` |
| 14 | `utime` | 11 | `ticks` (with `stime`) |
| 15 | `stime` | 12 | `ticks`, `system_ticks` |
| 18 | `priority` | 15 | `Process::priority` (20 + nice for normal tasks, negative for real-time) |
| 19 | `nice` | 16 | `Process::nice` |
| 20 | `num_threads` | 17 | `Process::threads` |
| 22 | `starttime` | 19 | `Identity::start` |
| 24 | `rss` | 21 | `Process::memory` after the page multiplication |
| 39 | `processor` | 36 | `Process::core`, the CPU the **main thread** last ran on; None if the field is absent |
| 41 | `policy` | 38 | `Process::policy` through the `sched.h` numbering (0 normal, 1 FIFO, 2 round robin, 3 batch, 5 idle, 6 deadline) |

Two consequences. `core` is where one thread was at the instant of the read, not where the process lives; a
multithreaded process occupies many cores and this field names one of them. And `priority`, `nice` and `policy`
are likewise the main thread's. `Process::started` is not in `stat`: it is `btime` from `/proc/stat` (read once,
at `Sampler::new`) plus `starttime / hz`.

A process that vanishes between the directory listing and the read is skipped, silently; that is a race, not a
gap, and `Sampler::unreadable` is always 0 on Linux because every `stat` file is world-readable (unless the
machine mounts `/proc` with `hidepid`, in which case other users' pids are simply not listed; isotop has no
special handling for that).

### `/proc/<pid>/io`

Cumulative, per process (summed over threads):

| key | meaning (proc(5)) | isotop |
|---|---|---|
| `read_bytes`, `write_bytes` | bytes the process caused to be fetched from, or sent to, the storage layer | `IoBytes`: the storage I/O rates and `Process::written` |
| `rchar`, `wchar` | bytes passed through `read`-family and `write`-family syscalls, whether or not they ever touched storage (page cache, pipes, sockets) | `SyscallCounts::read_chars`, `write_chars` |
| `syscr`, `syscw` | the number of those syscalls | `SyscallCounts::reads`, `writes` |

`write_bytes` is accounted when a page is dirtied, not when it is written back, so a short-lived temporary file
can count and never reach the disk (`cancelled_write_bytes` is the correction; isotop does not read it). The file
is readable only by the process's owner and root, so for other users' processes the read fails, `io` is `None`,
and every I/O rate of that process is `None` rather than 0.

### `/proc/<pid>/cmdline` and `cgroup`, and why they are cached

The command line is the process's argument vector, NUL-separated; isotop replaces the NULs with spaces and trims.
It is empty for kernel threads and zombies. `/proc/<pid>/cgroup` lists the cgroup of each hierarchy; on a unified
(v2) system there is one line, `0::/system.slice/cron.service`. From it isotop derives three things:

* `Process::cgroup`, the v2 path (the line beginning `0::`), empty for the root `/`;
* `Process::group`, the last path component that ends in `.service` or `.scope`, else the last non-empty
  component (`cgroup_name`); `"ungrouped"` if unreadable;
* `Process::kind`: `Container` if the path contains `docker`, `containerd`, `libpod`, `machine.slice`, `kubepods`
  or `lxc`; `Session` if it contains `user.slice` or `/user@`; otherwise `System` (`classify`). Kernel threads
  keep `Kind::Kernel` and no cgroup.

Neither file changes in the steady state, and reading both for every process on every sample would double the
number of file reads, so `Sampler` keeps a `Texts` record per `Identity` and re-reads it only when `reread` says so: when nothing
is cached, **when `comm` is no longer the cached `comm`** (an exec renames the process), and once every
`TEXT_REFRESH = 10` samples, staggered by `pid` so the re-reads spread over ten samples instead of arriving
together (`cmdline_is_reread_after_exec_and_cached_otherwise` checks that exactly one of ten samples is due for a
given pid and that the ten pids 0 to 9 are due on ten different samples). The staggered refresh also catches what
a `comm` check cannot: a `setproctitle` that rewrites the arguments, a move between cgroups, and an exec to a
program with the same name. The cache holds only the processes of the latest sample, so it cannot grow.

That cache is the Linux half of **exec detection**. A rename is the only exec signal the stat file gives; the
[life-event machinery](Theory-Life-Events) uses it only when the proc connector, which reports every exec
exactly, is not active.

## 3. System-wide readings

`Sampler::cpus(dt)` and its neighbours run after the process sweep in the same sample.

**Per-CPU load, `/proc/stat`.** Each `cpuN` line has the cumulative ticks `user nice system idle iowait irq
softirq steal guest guest_nice`. isotop sums the first eight columns as `total` and takes `idle + iowait` as
idle, so `busy = total − idle`. The last two columns are left out because guest time is already counted inside
`user` and `nice`; adding them would count it twice. Treating `iowait` as idle is a convention: it is idle time
during which a task was blocked on I/O. Over an interval,

$$
\text{busy} \;=\; \operatorname{clamp}\!\Big(\frac{\Delta\text{busy}}{\Delta\text{total}},\,0,\,1\Big),
$$

and it is `None` on the first reading, when no ticks elapsed, or when a counter went backwards.

**Run queue, `/proc/schedstat`.** Each `cpuN` line carries, after the name, nine numbers (version 15). Field 8 of
the line, `WAIT_FIELD = 7` as an index into the values, is the cumulative **nanoseconds that tasks spent
runnable but waiting for this CPU**. Its derivative is a time average: the total waiting time accrued per second
of wall time is the mean number of tasks that were waiting, because $\int n(t)\,dt / T = \bar n$. That is
`Cpu::wait`, computed as $\Delta\text{wait}_{\mathrm{ns}} / 10^{9} / \Delta t$ with the collector's `dt`
(measured at the start of the sample, not between the two reads, a small and unmeasured mismatch).

**Clock, cpufreq.** `/sys/devices/system/cpu/cpuN/cpufreq/scaling_cur_freq`, in kHz, divided by 1000 for MHz.
This is the frequency the cpufreq core last *requested* for that CPU, a software view, not a hardware readback.
Virtual machines usually have no cpufreq at all.

**Core kinds.** Intel hybrid CPUs expose the CPUs of each core type as ranges (`0-7,16`) in
`/sys/devices/cpu_core/cpus` and `/sys/devices/cpu_atom/cpus`; `core_kinds` reads both once, at start. Every
other machine (arm big.LITTLE included) has neither file and so reports `CoreKind::Unknown` for every CPU.

**Memory.** `/proc/meminfo`: `MemTotal` and `MemAvailable` (the kernel's estimate of what can be allocated without
swapping), in KiB, times 1024.

**Pressure stall information.** `/proc/pressure/cpu`, `memory`, `io` (Linux 4.20 and later, when the kernel
tracks PSI). The first line is `some avg10=… avg60=… avg300=… total=…`; isotop takes `some avg10`, the percentage
of wall time over the last ten seconds during which at least one task was stalled on that resource. The kernel
computes `avg10` as an exponential moving average updated every two seconds, so it is already a low-pass filter
with a ten-second time constant before isotop sees it: a pressure spike shorter than a few seconds is smeared.
Each of the three files is judged on its own: unreadable or unparsable reads 0 and is reported missing.

## 4. Cgroup v2 accounting (background thread)

`account` reads, for each cgroup path some process sits in (plus those wanted one cycle earlier, so a cgroup whose
last process was just killed is read once more for its `oom_kill`), the files in `/sys/fs/cgroup/<path>`:

| file | key | `Unit` field |
|---|---|---|
| `memory.current` | bytes now (a cgroup without this file is skipped) | `memory` |
| `memory.max` | a number or `max` | `memory_max`, `None` for `max` |
| `cpu.stat` | `usage_usec`, `nr_throttled` | `cpu` and `throttled`, rated |
| `cpu.max` | `quota period` or `max period` | `cpu_max` = quota / period × 100 |
| `cpu.pressure`, `memory.pressure`, `io.pressure` | PSI `some avg10` | `pressure[0..3]` |
| `memory.events` | `oom_kill` | `oom_kills` |
| `pids.current`, `pids.max` | processes | `pids`, `pids_max` |

CPU use is $100\,\Delta\text{usage\_usec} / 10^{6} / \Delta t$ with $\Delta t$ the wall time since the previous
read of that path (at least 0.1 s), `None` on the first reading or when the counter went backwards (a cgroup
recreated under the same path); `throttled` is `Δnr_throttled > 0`. A cgroup counts as unavailable when
`/sys/fs/cgroup/cgroup.controllers` lacks `memory` (cgroup v1, or no unified hierarchy) or when no process in the
sample sits in a cgroup below the root: inside a container every process may read as `0::/`.

## 5. Sockets: `/proc/net` and `sock_diag`

`platform::network()` builds a `Network`: which processes are connected, which connections leave the machine,
and per-connection statistics. It runs on the background thread.

1. **Who owns a socket.** For every `/proc/<pid>/fd/<n>` that is a link of the form `socket:[inode]`, record
   `inode → pid`. A socket inherited across `fork` appears in several processes; the first one the directory
   listing yields wins (`or_insert`), and the listing order is the filesystem's. Only descriptors isotop may read
   (its own user's, or everyone's as root) contribute, so a view's links are a lower bound.
2. **Loopback and local TCP.** `/proc/net/tcp` and `tcp6`: row fields 1 and 2 are the local and remote
   `address:port`, field 3 the state (`01` is `ESTABLISHED`), field 9 the inode. A local connection appears in
   the table twice, once from each end, with local and remote swapped. If the reversed pair exists, it is local
   and the pair of pids is a link (counted once, by comparing the two address strings); if it does not, the
   connection leaves the machine and increments that pid's `outside` count.
3. **Unix sockets.** A `sock_diag` dump (`NETLINK_SOCK_DIAG`, message type `SOCK_DIAG_BY_FAMILY` = 20, flags
   `NLM_F_REQUEST | NLM_F_DUMP`) of `AF_UNIX` with `UDIAG_SHOW_PEER` returns, per socket, its inode and its peer's
   inode; matching both through the owner table gives a pid pair.
4. **Per-connection statistics.** A second dump asks `AF_INET` and `AF_INET6` for established TCP sockets with the
   `INET_DIAG_INFO` extension, which attaches the kernel's `struct tcp_info`. The request and reply are decoded by
   byte offset:

   | structure | offset | bytes | meaning |
   |---|---|---|---|
   | `unix_diag_req` | 4 | `u32` | `udiag_states`, all ones |
   | | 12 | `u32` | `udiag_show`, `UDIAG_SHOW_PEER` (`0x4`) |
   | `inet_diag_req_v2` (56 bytes) | 0, 1 | `u8` | family, `IPPROTO_TCP` |
   | | 2 | `u8` | `idiag_ext`, `1 << (INET_DIAG_INFO − 1)` |
   | | 4 | `u32` | `idiag_states`, `1 << TCP_ESTABLISHED` |
   | `inet_diag_msg` reply | 6 | `u16` BE | remote port |
   | | 24 | 16 bytes | remote address (IPv4-mapped IPv6 is folded to IPv4) |
   | | 68 | `u32` | socket inode |
   | | 72… | attributes | `INET_DIAG_INFO` holds `tcp_info` |
   | `tcp_info` | 68 | `u32` | `tcpi_rtt`, smoothed round-trip time in microseconds |
   | | 120 | `u64` | `tcpi_bytes_acked`, cumulative |
   | | 128 | `u64` | `tcpi_bytes_received`, cumulative |

   A socket whose `tcp_info` is too short to hold the byte counters is left out rather than reported with zeros,
   because the next scan would difference a zero against a real value into an enormous rate
   (`a_tcp_socket_without_readable_statistics_is_left_out_not_counted_as_zero`). Netlink messages are 4-byte
   aligned (`(length + 3) & !3`) and the multipart reply ends at `NLMSG_DONE`.

If a `sock_diag` request fails, the corresponding part of the `Network` is empty and nothing is flagged missing;
Linux never inserts `Source::Sockets` or `Source::SocketTraffic` into `missing`.

## 6. Open files and file locks (background thread)

* **Open files**, `files.rs`. `scan_table` lists `/proc/<pid>/fd` and, for each descriptor, reads the link and, if
  the target begins with `/` (so not `socket:`, `pipe:` or `anon_inode:`) and not `/memfd:`, calls `statx` on the
  link, which reaches the open file even after it was deleted. The flag `AT_STATX_DONT_SYNC` tells network and FUSE
  filesystems to answer from cached attributes instead of asking their server, so a hung mount cannot block the
  thread. A regular file counts once by `(device, inode)` however many descriptors it has. A file whose link
  count is 0 (`stx_nlink == 0`) *and* whose target ends in ` (deleted)` is counted as deleted-but-open and its
  size is recorded: the kernel cannot free its blocks until the last descriptor closes. The scan is bounded
  (4096 descriptors examined and 16 384 listed per process, 65 536 listed per scan), resumes from the pid after
  the last one it reached, and past the examined bound **estimates** the open count in proportion
  ($\text{open} + \text{beyond}\cdot\text{open}/\text{examined}$), never extrapolating deleted files.
* **File locks**, `/proc/locks`. One line per held lock, `POSIX ADVISORY WRITE 1234 08:01:131 0 EOF`, followed
  by `->`-marked lines for the requests it blocks. The pid is the fourth field after the marks. Each waiter is
  mapped to the holder of the lock line above it. OFD locks belong to an open file description, not to a
  process, and report pid `-1`; a holder in another pid namespace reports `0`. Neither can be named, and
  `Blocker::Unnamed` records that something blocks without saying what.

## 7. NVIDIA memory through `dlopen`

`nvml.rs` loads `libnvidia-ml.so.1` with `dlopen` at run time, so isotop builds and runs without any NVIDIA
software. If the library or any of the six entry points it needs is absent, `Gpu::load` returns `None` and the
background cycle reports `Source::GpuMemory` missing. A laptop runtime-suspends an idle discrete GPU, and
querying NVML would wake it, so `awake` first scans `/sys/bus/pci/devices` for a device with vendor `0x10de`,
class `0x03…` (display controller) and `power/runtime_status` other than `suspended`, and only then initialises
NVML, queries the graphics and compute process lists (up to 512 entries each), and shuts NVML down again, so no
NVML session outlives a sample. `NVML_VALUE_NOT_AVAILABLE` (`u64::MAX`) becomes `GpuMemory::Unreported`, which is
distinct from "no entry" (the process does not use the GPU).

## 8. What is missing, and what says so

`Sampler::missing` returns the union of what this machine never has (`fixed_gaps`, computed at start) and what
the last readings could not get (`cpu_gaps`), plus `Cgroups` when no process sat in a cgroup below the root.
`Snapshot::missing` also holds the per-file pressure gaps and what the background thread found absent.

| `Source` | missing on Linux when | where |
|---|---|---|
| `CpuPressure`, `MemoryPressure`, `IoPressure` | the corresponding `/proc/pressure/*` file is unreadable or unparsable | `pressure` |
| `CpuClock` | no CPU has a `scaling_cur_freq` reading (typical in virtual machines) | `cpu_gaps` |
| `RunQueue` | `/proc/schedstat` is unreadable, or its `cpuN` lines have fewer than eight values (older versions) | `cpu_gaps` |
| `CoreKinds` | neither hybrid sysfs file names a CPU | `fixed_gaps` |
| `Cgroups` | `cgroup.controllers` has no `memory`, or no process is in a v2 cgroup below the root | `fixed_gaps`, `missing` |
| `GpuMemory` | `libnvidia-ml.so.1` cannot be loaded or lacks an entry point | background cycle |
| `FileLocks` | `/proc/locks` is unreadable | background cycle |

Never missing on Linux: `LastCpu` (the `processor` field), `SocketTraffic`, `SyscallIo`. Absent *values* are `None`
in the model, never zero, with one exception the pressure percentages: an unreadable pressure file reads 0 and the
`Source` in `missing` is the only thing that says so (the status line prints `n/a`).

## 9. What the numbers do not say

* **A sweep is not a snapshot.** Reading *n* processes takes *n* times the cost of a read; the first and last
  process are measured at different instants. Counters of one process come from files read microseconds apart, and
  `Collector::sample` divides all of them by one `dt` taken before the sweep. The smear is small against a one-second
  interval and large against a hundred-millisecond one.
* **`core` is a single thread's last CPU**, an instantaneous gauge. A busy multithreaded process shows one core.
* **`rss` is not a footprint.** It double counts shared pages across processes and ignores swapped-out ones.
* **`system_share` is statistical**, and every CPU percentage has a ±1 tick quantisation floor.
* **Other users' processes** are fully visible in `stat` (name, parent, CPU time, memory) but their `io` and
  `fd` tables are not, so their I/O rates, open files and sockets read `None` or are absent unless isotop runs as
  root.

## Where in the code

* [`platform/linux/mod.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/mod.rs): `Sampler`,
  `parse_stat`, `parse_io`, `Texts`, `reread`, `TEXT_REFRESH`, `cpus`, `cpu_gaps`, `fixed_gaps`, `core_kinds`,
  `pressure`, `meminfo`, `account`, `cgroup_name`, `classify`.
* [`platform/linux/net.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/net.rs): `sample`,
  `socket_owners`, `diag`, `unix_peer`, `tcp_socket`, `attributes`.
* [`platform/linux/files.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/files.rs):
  `FileScan`, `scan_table`, `classify`, `estimate`, `parse_locks`.
* [`platform/linux/nvml.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/nvml.rs): `Nvml`, `awake`.
* [`platform/mod.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/mod.rs): the `Sampler` contract,
  `RawProcess`, `Source`, `Sources`.
* [`model.rs`](https://github.com/tfpickard/isotop/blob/main/src/model.rs): `Collector::sample` and `measure`, which
  consume `RawProcess`.

## References

* `proc(5)`, the Linux manual page: the fields of `/proc/<pid>/stat`, `io`, `cgroup`, `/proc/stat`, `schedstat`.
* `time(7)`: the software clock, `HZ`, jiffies and `USER_HZ`.
* `netlink(7)` and `sock_diag(7)`: the socket interfaces used in section 5.
* Linux kernel documentation: `Documentation/scheduler/sched-stats.rst` (the `schedstat` fields),
  `Documentation/accounting/psi.rst` (pressure stall information) and
  `Documentation/admin-guide/cgroup-v2.rst` (the cgroup v2 files).
* `statx(2)`, for `AT_STATX_DONT_SYNC`.
