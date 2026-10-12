# Theory: Life events

A sampler sees who is alive *at the instant it looks*. Births, deaths and execs are things that happen *between*
looks. isotop has two ways to learn about them: the kernel's own notifications (the Linux proc connector and
taskstats, macOS `kqueue`) and **sample diffing**, comparing this sample's population with the last one's. The
first is exact and detailed but needs the kernel's cooperation and sometimes a privilege; the second always works
and misses what lived and died between two samples. `Life` in `model.rs` merges both into one ordered list,
`Snapshot::life`, in which **each exit appears once and is never revised**. This page explains the kernel
mechanisms, the byte layout isotop decodes, the merge, the queue between the event thread and the frame loop, the
privilege rules, and (found while writing it) one edge where the "once" has a time limit.

## Why events at all

Take sampling instants $\Delta t$ apart and a process that lives $L$ seconds, born at a random moment. It is
alive at some sampling instant, and so *seen*, with probability $\min(L/\Delta t, 1)$ (the instant must fall inside
its life). For exponentially distributed lifetimes with mean $m$ the expected fraction seen is

$$
\mathbb{E}\big[\min(L/\Delta t,\,1)\big] \;=\; \frac{m}{\Delta t}\Big(1 - e^{-\Delta t/m}\Big).
$$

A shell command, a compiler child or a `sh` helper that lives a tenth of a second is on the books at a once-a-second
instant 10 % of the time; if its mean life equals $\Delta t$, 63 %.

![Fraction of processes ever seen by a sampler against mean lifetime over sampling interval](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/life-survival-sampling.svg)

This is the Nyquist problem for point events. Sample diffing cannot recover what it never saw; the proc connector
reports every fork and exit as it happens. A view that tells a *story* of births and deaths (the
[Necropolis](View-Necropolis), whose whole subject is the dead) needs them. Views that draw only the population
(everything else) use sample-to-sample arrivals ([History](Theory-History) has `changes`) and need no events.

## 1. The kernel's side (Linux)

Both mechanisms are **netlink** sockets: datagrams between the kernel and a process, addressed by a protocol
number and, for multicast, a group.

### The proc connector

The *process events connector* (`NETLINK_CONNECTOR`, index `CN_IDX_PROC = 1`, value `CN_VAL_PROC = 1`) multicasts a
message from inside the kernel's fork, exec and exit paths. A listener subscribes by sending
`PROC_CN_MCAST_LISTEN`. Events are per **task** (thread), so a thread creation is also a "fork"; isotop reports
processes, so it ignores thread events (`pid != tgid`). The message, as `include/uapi/linux/cn_proc.h` lays it out and
as `Decoder::connector` reads it (kernel byte order):

| offset in datagram | size | field |
|---|---|---|
| 0 | 16 | `nlmsghdr` (length, type `NLMSG_DONE`, flags, sequence, port) |
| 16 | 20 | `cn_msg`: idx, val, seq, **ack**, len, flags |
| 36 | 4 | `proc_event.what`: `PROC_EVENT_FORK` 0x1, `PROC_EVENT_EXEC` 0x2, `PROC_EVENT_EXIT` 0x80000000, `PROC_EVENT_NONE` 0 (acknowledgement) |
| 40 | 4 | cpu |
| 44 | 8 | **timestamp in nanoseconds**, `CLOCK_MONOTONIC` |
| 52 | 4 each | `event_data`, as words: fork = parent pid, parent tgid, child pid, child tgid; exec = pid, tgid; exit = pid, tgid, **exit_code**, exit_signal, parent pid, parent tgid |

The exit code is a *wait status*: the low 7 bits are the terminating signal (0 for a normal exit), bit 7 says a core
was dumped, and bits 8 to 15 are the exit status. `Cause::of_status` is exactly the `WIFEXITED`/`WIFSIGNALED`
decoding: `Code(n)`, or `Signal { number, core }`. The connector is a *push* interface: it carries no cpu time, no
memory, and no command line.

### Taskstats

*Taskstats* (generic netlink, the `TASKSTATS` family whose numeric id is looked up at start through the control
family `GENL_ID_CTRL`) is the kernel's per-task accounting struct delivered at exit. A listener registers for a CPU
mask (`TASKSTATS_CMD_ATTR_REGISTER_CPUMASK`, here "all possible CPUs", read from `/sys/devices/system/cpu/possible`)
and receives the record of every task that exits on those CPUs: one record **per thread**, and for the last thread
of a process an extra aggregate. The record carries what the connector lacks: command name (`ac_comm`), start
(`ac_btime`/`ac_btime64`), run time (`ac_etime`, µs), user and system CPU time (µs), peak resident memory
(`hiwater_rss`, KiB), storage bytes read and written, voluntary and involuntary context switches, the `AFORK` flag
(forked, never exec'd), and the exit code. isotop decodes it by byte offset from `taskstats.h` as of Linux 6.8
(`TASKSTATS_VERSION` 14), where "new versions only append fields", checking the version word before using later
fields:

| field | offset | notes |
|---|---|---|
| `version` | 0 | selects the layout |
| `ac_exitcode` | 4 | same wait status |
| `ac_flag` | 8 | `AFORK` 0x01, `AGROUP` 0x20 (version 12 and later: last thread of the group) |
| `ac_comm` | 80, 32 bytes | NUL-terminated |
| `ac_pid` | 128 | the thread id |
| `ac_btime` / `ac_btime64` | 136 / 344 | seconds since the epoch; the 64-bit form from version 10 |
| `ac_etime`, `ac_utime`, `ac_stime` | 144, 152, 160 | microseconds |
| `hiwater_rss` | 200 | KiB |
| `read_bytes`, `write_bytes` | 248, 256 | |
| `nvcsw`, `nivcsw` | 272, 280 | context switches |
| `ac_tgid` | 368 | version 12 and later |

`Decoder::record_message` **sums the thread records of a process** into one `ExitRecord` (CPU, I/O and switch counts
add; `hiwater_rss` and `etime` take the maximum; `btime` the minimum), takes the command and `AFORK` from the
leader's own record, and emits it when the last thread's record arrives (the `AGROUP` flag where the version has
it; before it, the aggregate message or the leader's record with no known thread left). Which thread belongs to
which process, before `ac_tgid` existed, comes from the connector's thread forks.

### Why `CAP_NET_ADMIN`, and why isotop never escalates

Netlink families can mark an operation as administrative; the kernel then requires `CAP_NET_ADMIN` in the *initial*
user namespace before it will honour it. Taskstats' registration is such an operation, always. The proc connector
required it too; isotop's source and README state that it needs no privilege from Linux 6.6 on (not re-verified
against kernel history for this page). Both only work for a process in the **initial PID and user namespaces**
(`initial_namespaces` checks the `/proc/self/ns/pid` and `user` links against the fixed inode numbers
`pid:[4026531836]` and `user:[4026531837]`); inside a container only sample diffing remains.

`open_events` is written never to escalate: it opens each source once, takes what the process may open as it is,
and returns `None` with no thread if neither opens. An unprivileged run gets the connector on a new kernel and
sample diffing otherwise; you can *choose* to grant the binary the capability (`setcap cap_net_admin+ep`), and
isotop will use it, but it will not ask, `sudo` itself, or retry.

### The subscription handshake

`open_connector` binds the socket to the multicast group, sends `PROC_CN_MCAST_LISTEN` with a nonce in the `seq`
slot and waits (up to 256 messages, with a 200 ms receive timeout) for the kernel's acknowledgement, a
`PROC_EVENT_NONE` message whose `ack` field is nonce+1 and whose first data word is 0 (success). Outside the initial
namespaces the request is ignored and no acknowledgement ever comes, so the socket is dropped. The receive buffer is
requested at 4 MiB (`SO_RCVBUF`; the kernel caps it at `rmem_max`), because a burst of forks outruns the default.
`open_taskstats` asks the control family for the `TASKSTATS` id, then registers with `NLM_F_ACK` and waits for
error code 0.

### The thread

`isotop-events` polls both sockets with a 200 ms timeout, so it notices within 200 ms that nobody holds the queue
any more and exits, closing its sockets. It reads with `MSG_DONTWAIT` in a loop until the socket is empty; an
`ENOBUFS` from `recv` means the kernel dropped messages because the socket's buffer was full, and is counted
(`LifeQueue::overrun`).

## 2. macOS: `kqueue`

macOS has no connector and no taskstats. `kqueue` with `EVFILT_PROC` can watch **one pid at a time**, any process of
the same user, child or not, for `NOTE_EXEC` and `NOTE_EXIT`. `isotop-events` therefore watches every pid in the
latest sample (the collector shares the `pid → Identity` map; each loop it registers new ones with `EV_ADD | EV_CLEAR`
and forgets those that left the sample) and stamps each event with `monotonic_ns()` when it receives it. Consequences:

* **No fork events.** A fork names no child here, so births come from sample diffing, at the time of the sample that
  saw them.
* **No exit status or record.** `NOTE_EXITSTATUS` is reported for children only, so every kqueue exit has
  `Cause::Unknown`.
* **Only the sampled are watched.** A process that lives and dies between two samples was never in a sample and was
  never watched, so kqueue cannot see it. (On Linux the connector can.)
* **Other users' processes** are refused by the kernel and are simply not watched.
* The exec itself is exact: a macOS process also carries a pid *version* that changes on every exec
  ([macOS sampling](Theory-Sampling-macOS)), which sample diffing uses as a backstop.

## 3. From observations to events

The event thread produces `Observed { at, source, what }`: `at` in nanoseconds of `CLOCK_MONOTONIC`, the kernel's
timestamp where the source gives one (the connector) and the receive time otherwise; `what` is `Fork`, `Exec`, `Exit`
or `Record` (a taskstats record). The collector turns them into `LifeEvent`s: `at` in seconds of
`Snapshot::elapsed` (`seconds_since(origin_ns, at)`, where the collector captured `CLOCK_MONOTONIC` at its
construction), a `kind`, the `source`, and an `estimated` flag.

**Naming a process.** An observation carries a pid; the model needs an `Identity`. `Life::merge` names it from, in
order: a start the source gave (the connector reads `/proc/<pid>/stat` for the parent and child at the time of the
event); the previous sample's population plus forks reported this interval; this sample. If the pid is in none of
those the event is **estimated**: a forked child already gone before its `stat` could be read gets the start its
fork's *timestamp* implies, converted by the `Clock` into ticks since boot (monotonic time plus the suspended time,
times USER_HZ), and a process nothing names gets start 0. **An estimated identity never matches a sampled process**,
so an estimated event can never be joined to a wrong one.

### The merge

`Life` keeps, between samples, `reported` (what has been emitted, by kind and `Identity`, with its time),
`exits` (connector exits waiting for their taskstats record), `records` (taskstats records waiting for their
exit) and `forked` (forks of the last interval, so a following exit can name the child). Per sample:

1. **Walk the observations in arrival order.**
   * A **fork** names parent and child, remembers the child, and is emitted.
   * An **exec** is emitted.
   * An **exit** is *dropped if that identity already exited or waits*; else, when the source is the connector and
     taskstats is active, it is **held** in `exits` for its record; else emitted at once.
   * A **record** is attached to the exit holding it if there is one; else, if the connector is active, held in
     `records`; else (taskstats alone) emitted as an exit with the record's own cause. A record whose begin time
     is more than one second off the identity it would name belongs to another process and is dropped.
2. **Expire.** After `RECORD_WAIT = 0.5 s` (of event time) whichever half is still alone is emitted alone.
   `attach` merges a held exit with its record: the event **keeps the connector's exact time and cause** and gains the
   record, and is re-labelled a taskstats event (the most detailed source).
3. **Diff**, but only if the previous sample was also taken under demand (so the first sample reports nothing):
   * a process in this sample and not the last, with no fork reported for it: a **birth**, at this sample's time,
     source `Diff`, parent taken from this sample (or the last if the parent is gone);
   * a process in both whose exec marker changed and that no event reported: an **exec**. On macOS the marker is the
     pid version; on Linux it is the process *name*, and only while the proc connector is not active (the connector
     reports every exec, so a rename is then not an exec);
   * a process in the last sample and not this one, and not *claimed* (reported, or waiting in `exits` or `records`):
     an **exit**, `Cause::Unknown`, source `Diff`, at this sample's time.
4. **Prune** `reported` of entries older than `LIFE_MEMORY = 10 s`, and sort the sample's events by `at`.

Preference when two sources saw an exit: taskstats (cause, time, record) over the connector (cause, time) over kqueue
(time) over diff (existence). The tests pin the cases: connector and diff report one exit; taskstats and connector one
exit with the record at the connector's time; an exit that came before its record waits and is delivered later while
diffing leaves it alone; a late record after the exit was reported alone is dropped; a connector exit that happened
*after* the scan of the sample that still lists the process is not repeated by the next sample's diff.

### The exit invariant, and its limit

**One exit per `Identity`, never revised** is enforced by `reported`: a second exit for the same identity is
discarded, and a reported event is never edited (attachment happens *before* reporting, while the exit is held for
at most half a second). It is what lets a consumer treat `Snapshot::life` as an append-only log.

There is a limit, found by reading `merge` and confirmed with a probe test (a scratch copy of the repository, not
committed): the guarantee is bounded by `LIFE_MEMORY`. A **zombie** (a process that has exited and not been reaped)
stays in `/proc` and in every sample as state `Z`. The connector reports its exit when it *becomes* a zombie; if its
parent reaps it more than 10 s later, the entry in `reported` has been pruned, the process is in the last sample and
not this one and is not claimed, and diffing emits a **second** exit for it, `Cause::Unknown`, source `Diff`, at the
reaping sample. In the probe an exit reported at 1.5 s for a process still listed at 2 s to 19 s produced
`Exit { cause: Unknown, source: Diff }` at 20 s, while the same reaping within 8 s produced nothing. The same
window opens at `--sample-ms 10000`. Consumers that need exactly-once guard it themselves: the Necropolis keeps its own
`mourned` set and ignores a repeated exit. A reported-set that outlives the longest zombie, or diffing that skips
processes that were already `Z`, would close it; it is a boundary of the current design, not a measurement of how
often it occurs.

## 4. The queue and what "dropped" means

The event thread and the frame loop share a `LifeQueue`: a mutex around a `Vec<Observed>` and a `dropped` count.

* `LifeQueue::CAPACITY = 4096`. `push` appends while the vector is shorter; otherwise it **drops the new
  observation** and increments `dropped` (saturating). The *earliest* 4096 are kept (a test checks this).
* `overrun` also increments `dropped`, for an observation lost *before* it reached the queue (the kernel had more
  for the socket than its buffer held, `ENOBUFS`).
* `drain` returns everything queued in arrival order and the count dropped since the last drain, resetting both. The
  collector calls it once per sample and exposes the count as `Snapshot::life_dropped`.

"Dropped" therefore means an event the stream lost, not a process that did not exist. What it costs depends on the
loss: a lost exit is later found by diffing as an exit with `Cause::Unknown` (existence kept, detail lost); a lost
fork becomes a birth at sample time; a lost exec is lost, on Linux, unless the process was renamed. The queue is
sized for bursts (a build forking thousands of processes in a second) between samples a second apart; a larger
`--sample-ms` makes overflow more likely.

## 5. Lifecycle

Events are read **only while a view demands them** (`Source::LifeEvents`; today the Necropolis). On the first sample
under demand `Life::start` opens the sources once (`tried` prevents retries until the demand ends). While demanded
`Snapshot::sources` lists `LifeEvents`, `SampleDiff` and the mechanisms that opened, and `Snapshot::missing` lists
`ForkEvents` when the proc connector is not delivering and `ExitRecords` when taskstats is not. When the demand ends,
`Life::default()` replaces the state, dropping the queue, which ends the thread within 200 ms and closes the
sockets. Deaths while another view is shown are therefore not recorded. The demand itself is described in
[the background sampler](Theory-Background-Sampler-and-Demand#5-demand-which-optional-sources-are-read).

`--demo` runs no thread: `demo_events` computes the events from the churn schedule and the snapshot carries them
directly ([Model and rates](Theory-Model-and-Rates#the-demo-workload)); under the Necropolis's `mortality` option one
exit in ten is delivered as a diff event with no cause, to exercise that path.

## 6. What a viewer can infer

* An exit with a signal, a status and a record came from the connector and taskstats; "died between samples" is
  diffing and means *only* that the process was gone, not why, and not when within the interval.
* A fork or exit with an exact time is a kernel timestamp (Linux) or the instant isotop's thread was woken (macOS
  kqueue); a diff event is stamped with the *sample's* time, up to one interval late.
* Absence of an event proves nothing about a process that lives and dies inside one interval on macOS, or when the
  queue overflowed.
* An estimated identity is a guess named as such, and is never drawn as a known process.

## Where in the code

* [`platform/linux/events.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/events.rs):
  `open_events`, `open_connector`, `open_taskstats`, `Decoder`, `parse_record`, `Clock`, `field` offsets.
* [`platform/macos/events.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/macos/events.rs): the
  `kqueue` thread.
* [`platform/mod.rs`](https://github.com/tfpickard/isotop/blob/main/src/platform/mod.rs): `LifeEvent`, `LifeKind`,
  `LifeSource`, `Cause`, `ExitRecord`, `Observed`, `Observation`, `LifeQueue`.
* [`model.rs`](https://github.com/tfpickard/isotop/blob/main/src/model.rs): `Life` (`merge`, `expire`, `diff`),
  `attach`, `RECORD_WAIT`, `LIFE_MEMORY`, `Collector::sample` (the demand and drain), `demo_events`.
* [`necropolis.rs`](https://github.com/tfpickard/isotop/blob/main/src/necropolis.rs): the consumer.

## References

* Linux kernel documentation: `Documentation/driver-api/connector.rst` (the connector) and
  `Documentation/accounting/taskstats.rst` (taskstats and its delivery); the uapi headers `cn_proc.h`, `connector.h`,
  `genetlink.h`, `taskstats.h`.
* `netlink(7)`, `capabilities(7)`, `namespaces(7)` and `wait(2)` (the status macros).
* `kqueue(2)` in the macOS and BSD manuals.
