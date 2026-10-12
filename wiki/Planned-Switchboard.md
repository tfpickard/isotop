# Planned: Switchboard

> **Planned, not yet implemented.** Specified in [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md) and tracked in issue [#48](https://github.com/tfpickard/isotop/issues/48).

Hardware interrupts as a 1940s cord telephone exchange: a tall wooden board of jacks, one per interrupt line, each with a small lamp that flickers at the line's rate; cords that hang on true catenaries from each jack to the operator (CPU) that serviced most of its interrupts; a row of operators at their seats, P-cores first; and a smaller second board for softirqs. When irqbalance moves a line, the cord is lifted, swung over and replugged. No processes are drawn at all. This page is the design of that view, and a tour of the machinery it displays: interrupt controllers, `/proc/interrupts`, softirqs, affinity, and the queueing mathematics that telephone engineers invented for exactly this kind of switchboard.

![Reference mockup of the switchboard view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/mockups/switchboard.jpg)

*`docs/mockups/switchboard.jpg`, from the Python prototype `docs/mockups/code/switchboard.py` with a mocked `/proc/interrupts` sample (rates, the irqbalance move at t = 3 s and the ring schedule are mocked). A visual target, not an isotop render.*

## 1. The system being modelled

### 1.1 The manual exchange

In a cord switchboard each subscriber's line ends at a **jack** on the board with a **line lamp** above it. In a common-battery exchange the lamp lights when the subscriber lifts the handset. An operator seated in front of the board answers by inserting the **answering plug** of a **cord pair** into that jack and throwing a key to speak; she asks for the number and inserts the other plug of the pair, the **calling plug**, into the called party's jack. Each cord has a **supervisory lamp** that tells her when its end hangs up, so she can pull the plugs and free the cord. Operators, cords and trunks to other offices are the scarce resources; jacks are plentiful.

Almon Strowger patented an automatic step-by-step exchange in 1891 (the first public installation followed in La Porte, Indiana in 1892) to take the operator out of the loop; the mockup's references are the manual boards that came before it, such as Western Electric's PBX boards (the brief names the No. 555).

### 1.2 The analogy, and where it is loose

| Exchange | Machine |
|---|---|
| subscriber line, its jack | an interrupt line (a row of `/proc/interrupts`) |
| off-hook, line lamp lights | the device asserts the line |
| operator at a position | a CPU |
| answering cord plugged in | the line is being serviced by that CPU |
| operator's telephone ringing | a rescheduling IPI (`RES`) |
| trunk to another office | an inter-processor interrupt |

The analogy is loose in three ways, which the design handles. A line lamp is a **level** (pending or not); the view's lamp shows the **rate**. A cord in a real exchange joins two parties; in the view a cord joins a line to the **CPU that handled it**, and nothing ever joins two CPUs, because **the sender of an inter-processor interrupt is not recorded** anywhere the kernel exposes. And a real operator serves one call at a time; a CPU services many.

## 2. Theory: interrupts

### 2.1 From wires to messages

A device signals the CPU with an **interrupt**; the CPU stops what it was doing, runs a handler, and returns. The original PC had two cascaded 8259 programmable interrupt controllers, fifteen shared wires. Modern x86 has a **local APIC** in every CPU and one or more **I/O APICs** that route the legacy wires to a chosen CPU's local APIC. PCI devices increasingly use **MSI and MSI-X**: instead of pulling a wire, the device performs a memory write to a special address whose data names the vector, so each device can own many vectors with no shared line. That is why one NVMe disk appears as `nvme0q0` to `nvme0q8` (one queue per CPU-ish) and a NIC as `eth0-rx-0`, `eth0-tx-1`: the multiqueue hardware and the multi-vector interrupt model were built for each other.

### 2.2 `/proc/interrupts`

Each numeric row is `IRQ: count-per-CPU... chip hwirq-and-trigger device-names`. For example an MSI-X vector reads like `147: ... IR-PCI-MSIX ... eth0-rx-2`. The columns follow the `CPUn` header (offline CPUs are left out of the header, so the view must map columns by header, not by position, as B4 specifies). Besides numeric rows there are **named rows** with a description instead of chip data. On x86 these are `NMI` (non-maskable), `LOC` (local APIC timer), `SPU` (spurious), `PMI` (performance monitoring), `IWI` (irq work), `RTR`, `RES` (rescheduling IPIs), `CAL` (function-call IPIs), `TLB` (TLB shootdowns), `TRM`, `THR`, `DFR`, `MCE`, `MCP`, `HYP`, `ERR`, `MIS`, `PIN`, `NPI`, `PIW` (the list in issue [#31](https://github.com/tfpickard/isotop/issues/31)); some rows (`ERR`, `MIS`) have a single total rather than a count per CPU. On arm64 there is no `LOC` row; inter-processor interrupts appear as `IPI0:` to `IPI6:` and the timer is an ordinary row such as `arch_timer`. Counts are cumulative since boot, so the view needs rates from deltas.

### 2.3 Top halves and softirqs

The handler itself (the "top half") does the minimum and defers the rest. Deferred work runs as a **softirq**, `tasklet`, or workqueue item. The kernel has a fixed list of softirq vectors, all ten shown on `/proc/softirqs`: `HI`, `TIMER`, `NET_TX`, `NET_RX`, `BLOCK`, `IRQ_POLL`, `TASKLET`, `SCHED`, `HRTIMER`, `RCU`. Network receive uses **NAPI**: the first packet interrupt masks the queue and schedules polling in `NET_RX`, which processes a budget of packets, so a flood of packets becomes a polled stream rather than a storm of interrupts. When softirqs run too often, a per-CPU `ksoftirqd` thread takes them over so that they compete fairly with user threads.

### 2.4 Affinity and irqbalance

Each numeric IRQ has `/proc/irq/N/smp_affinity` (a bitmask), `smp_affinity_list` and, where the kernel exposes it, `effective_affinity_list`: the CPU or CPUs that really receive it, which can be a subset of the requested mask (on x86 a vector is delivered to one CPU of the mask). The `irqbalance` daemon rewrites affinities from time to time to spread load; drivers with one vector per queue also pin each vector to a CPU. The view's cord swing, "irqbalance moved this line", is the observable edge of that rewrite.

## 3. Theory: the honest mathematics of a switchboard

The people who studied exchanges a century ago asked: given $N$ operators (or trunks) and calls arriving at random, how often is a caller blocked, or how long does he wait? A. K. Erlang answered in 1917 (*Elektroteknikeren*, "Solution of some problems in the theory of probabilities of significance in automatic telephone exchanges", English translation in the *Post Office Electrical Engineers' Journal* 1918).

### 3.1 Erlang B: calls cleared

Calls arrive as a Poisson process of rate $\lambda$; each holds a server for a random time of mean $1/\mu$; there is no queue: if all $N$ servers are busy the call is lost. The offered load is $A = \lambda/\mu$ erlangs. The number of busy servers is a birth-death chain. Balancing the flow between neighbouring states, $\lambda\, p_{k-1} = k\mu\, p_k$, gives $p_k = p_0 A^k / k!$, and normalising,

$$
B(N, A) = p_N = \frac{A^N / N!}{\sum_{j=0}^{N} A^j / j!}.
$$

Writing $S_n = \sum_{j \le n} A^j/j!$ and $B_n = (A^n/n!)/S_n$ gives $1/B_n = 1 + (n/A)(1/B_{n-1})$, the stable recurrence

$$
B_0 = 1, \qquad B_n = \frac{A\,B_{n-1}}{n + A\,B_{n-1}} .
$$

The result is **insensitive** to the distribution of holding times beyond their mean (it holds for any such distribution in this loss system), which is why it survived a century of changing technology. Computed from the recurrence (`docs/figures/switchboard-erlang.py`), checked against the closed form (B(10, 5) = 0.018385):

| Trunks $N$ | max load for 1 % blocking | utilisation at that load | max load for 5 % |
|---|---|---|---|
| 1 | 0.010 | 1 % | 0.053 |
| 2 | 0.153 | 7.6 % | 0.381 |
| 5 | 1.361 | 27 % | 2.218 |
| 10 | 4.461 | 44 % | 6.216 |
| 20 | 12.031 | 60 % | 15.249 |
| 50 | 37.901 | 75 % | 44.533 |
| 100 | 84.064 | 83 % | 95.240 |

Large pools run efficiently: a single trunk at 1 % blocking is 1 % busy, a hundred are 83 % busy.

### 3.2 Erlang C: calls queue

If blocked calls wait, the system is M/M/$N$ and the probability that an arrival must wait is

$$
C(N, A) = \frac{N\,B(N, A)}{N - A\,(1 - B(N, A))}, \qquad A < N,
$$

with mean waiting time $W_q = C(N,A)/(N\mu - \lambda)$. Pooling pays off: at utilisation 0.9 the probability of waiting is 0.90 for one server, 0.79 for four, 0.59 for sixteen and 0.31 for sixty-four, and the mean wait in units of the service time falls from 9.0 to 1.97, 0.37 and 0.049.

![Erlang B and Erlang C curves](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/switchboard-erlang.svg)

### 3.3 What carries over to interrupts, and what does not

Erlang's results are exact for Poisson arrivals and independent exponential (or arbitrary, for B) service. Interrupts are neither: network packets arrive in bursts, NICs and drivers **coalesce** interrupts, timers are periodic, and handler times are heavy-tailed. Interrupts are not blocked and cleared; they are masked and made pending, except where a finite queue overflows (a full NIC receive ring drops packets, which is the real "blocked call"). The honest use of Erlang here is as the story of why the view has the shape it has: a small number of operators (CPUs) pooled over a large number of lines, with a few lines (a busy NIC queue) accounting for most of the load, which is why the board has lamps of very different brightness and cords of very different thickness. The view makes **no queueing claim**.

## 4. Mapping

The realness contract: every element maps to a measurement or is declared decorative.

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Jack | one row of `/proc/interrupts` (numeric IRQ or MSI vector) | labelled with the device name; grouped by device key (name minus trailing queue suffix `[-:_]?(q\|rx-\|tx-\|TxRx-)?\d+$`) |
| Lamp brightness | per-CPU count delta over the sampled interval, summed | $\log_{10}(\text{rate}+1)/\log_{10} 8000$ for jacks, $/\log_{10}5000$ for softirqs, clamped to 1 |
| Lamp flicker | the same rate | event $k$ at $(k + \text{jitter}(\text{irq},k))/f$, $f = \min(\text{rate}, 0.9\,\text{fps})$; the inspector shows the true rate |
| Cord end at an operator | the CPU with the largest delta in the interval | a line with zero delta keeps its cord |
| Cord thickness | rate | $0.6 + 0.5\log_{10}(\text{rate}+1)$, clamped |
| Cord swing | `effective_affinity_list` change, or servicing CPU change persisting 2 samples | lift 0.35 s, arced smoothstep swing 1.05 s, seat 0.25 s |
| Cord colour | CPU kind | warm table for P-cores, cool for E-cores and Unknown |
| Operator | one CPU (P first, then E, then Unknown; up to 64 seats, above that each seat covers $\lceil n/64\rceil$ CPUs) | head and arm motion scaled by `Cpu.busy` |
| Clock hand at an operator | `LOC` (arm64: the `arch_timer` row) | one revolution per 6000 counts, interpolated between samples |
| Operator's telephone rattles | `RES` (arm64 `Rescheduling`) | while rate $\ge$ 1/s |
| Green lamp | `CAL` + `TLB` (arm64 `Function call`) | log brightness |
| Red lamp | an `NMI` delta | lit for 2 s |
| Softirq board | `/proc/softirqs`, 10 rows, one lamp per CPU | log brightness as above |
| Other named rows | not drawn; the README lists them | |
| Wood, chairs, operator figures | none | **decorative** |

## 5. Design: how isotop will realise it

* **Module and row.** `src/switchboard.rs`, `#[derive(Default)] struct Switchboard`, one `ViewInfo` row: `Backdrop::Dusk`, no view-local keys, no digit, opted out of the tour; `draw` returns 0 (no process is drawn; the header override reads like `48 lines | 41,203 irq/s | RES 2,114/s | 16 operators: 8 P, 8 E`). It reads only the B4 fields of `Snapshot`, so pause and rewind replay history.
* **Sources.** B4 (issue [#31](https://github.com/tfpickard/isotop/issues/31)): `Snapshot.interrupts`, `Snapshot.softirqs` and `/proc/irq/<n>/effective_affinity_list`, read on the existing **2 s** background pass, and only while the view is shown (`Demand`). Rates are therefore 2-second averages, and with the "two consecutive samples" rule a change of servicing CPU shows after about 4 s (my derivation from the two numbers).
* **Rates.** Per-CPU count delta divided by the interval; a counter that decreases reads 0 (never negative).
* **Jacks.** Seats last for the line's lifetime and freed seats are reused lowest-first (as `layout::Seats`; if its `Identity` key does not fit B9's line id, a small local equivalent, never a fake `Identity`). Groups are ordered by lowest IRQ number; columns = clamp(ceil(sqrt(3n)), 12, 16); a cap of 128 jacks (rows whose count never changed are hidden first; above the cap each device group of more than 8 vectors folds into one jack labelled like `nvme0q* (33)`).
* **Cords.** World-space ribbons (S3) with 24 segments on a true catenary solved by bisection in `switchboard::catenary(a, b, length, segments)` and cached until the endpoints move. Length = chord times a slack in 1.02 to 1.07 hashed from the IRQ id. For a cord between level endpoints a chord $h$ apart, the catenary $y = a\cosh(x/a)$ of length $L$ satisfies $2a\sinh(h/2a) = L$; bisecting on $a$ gives the sag, which is close to the parabolic estimate $h\sqrt{3(s-1)/8}$: for slack $s = 1.02, 1.045, 1.07$ the sag is $0.087h$, $0.132h$, $0.166h$ (computed). The cord colour crossfades and the lamp flutters while it is unplugged.
* **Clock.** The swing and lamp timing run on wall-clock time through `simulation::FixedStep`. Flicker is stateless and repeatable from `(irq, k)`: the same time gives the same lamps.
* **Picking.** Jacks, lamps and cords pick the line through B9's non-process pickables (`Thing`, `Card` in `src/render.rs`); the inspector shows name, IRQ, chip, rate, servicing CPU, affinity and effective affinity, last move, and the top 3 CPUs by share. Operators and softirq lamps are not pickable.
* **Text.** All device text passes through `terminal::clean_text`; labels go through `stage.places`.
* **Demo.** The prototype's 48 lines, 16 CPUs and softirq rates; `eth0-rx-2` moves between cpu3 and cpu9 at $t = 11.2 + 30k$ s; one NMI on cpu0 every 45 s; RES rate proportional to busy; a 3 s headless warm-up so `--time 12` shows a swing.
* **Budget.** About 4k triangles for the board (axis-aligned grain quads only), cords within $128 \times 24 \times 2$ triangles, lamp glows within 12 px at 1080p.
* **Dependencies** (ROADMAP index): P1, P2, S1, B4, B9, S2, S3, S6. In the tree at commit `192299b`: P1, P2, S1, S2, S6 and B9 are merged; B4 (no `Interrupt` type or `/proc/interrupts` reader) and S3 (no ribbons) are not.
* **Platforms.** Linux: all sources are unprivileged; each missing one is named in the strip (unreadable `/proc/interrupts`: dark board; unreadable affinity: only servicing-CPU changes swing cords; unreadable softirqs: dark softirq board; no timer row: the clocks stop). macOS: **unavailable**, because macOS exposes no unprivileged per-IRQ or per-CPU interrupt counters (`powermetrics` needs root, IOReport is private). `available()` returns false: Tab and the picker skip it silently, `--verbose` prints `skipping view switchboard: not supported on this platform`, and there is never a "not supported" screen. The module still compiles there.
* **No plugin system.** Compile-time module and `REGISTRY` row, like every view.

## 6. Reading the picture

* **Measured:** which lines exist and their per-interval rates (as 2-second averages), which CPU serviced the most of each, the affinity and its changes, the per-CPU timer, reschedule, call and TLB counts, and the softirq counts per CPU.
* **Derived:** lamp brightness (log of a rate), cord thickness, the swing (a state machine over two measured quantities), the clock hand (counts converted to an angle).
* **Decorative:** wood, chairs, operators' bodies, the exact cord slack, and the flicker's pattern: a lamp flickers at $\min(\text{rate}, 0.9\,\text{fps})$ events per second **by display convention** and its phase is a hash, so the lamp's rhythm is not the interrupt arrival process. A frame samples at most fps events per second; capping at 0.9 fps and jittering avoid the strobe aliasing that an exactly periodic flicker at the frame rate would show (a flicker at $f$ sampled at fps appears at $|f - n\cdot\text{fps}|$; at $f = 0.9\,\text{fps}$ that is a slow 0.1 fps beat, at $f = \text{fps}$ a frozen pattern). That is my inference about the choice, not a statement of the issue.
* **Cannot infer:** which CPU *sent* an inter-processor interrupt (not recorded); which process caused an interrupt (no per-process attribution; the issue lists it out of scope); a line's individual interrupt times; any queueing or latency.
* **Misreadings to avoid:** a cord shows the **plurality** servicing CPU, not the affinity mask; a line spread evenly over eight CPUs shows one arbitrary cord. A cord that does not move may be a quiet line, not a pinned one.

## 7. Try it

Nothing runs in isotop yet. When the view lands:

```sh
./target/release/isotop --demo --view switchboard --time 12 --width 1600 --output /tmp/switchboard.png --renderer cpu
```

(Tab and the `v` picker reach it on Linux.) To see the real tables the view reads:

```sh
head -3 /proc/interrupts; cat /proc/softirqs; cat /proc/irq/*/effective_affinity_list | sort | uniq -c
```

and to regenerate the queueing figure: `python3 docs/figures/switchboard-erlang.py`.

## 8. Where in the code

Existing infrastructure: `platform::Source` and `Sources` (`src/platform/mod.rs`), `status::missing_note` and `sources_note` (`src/status.rs`), `simulation::FixedStep` and `hash_identity` (`src/simulation.rs`), `layout::Seats` (`src/layout.rs`), `Thing`, `Card`, `Frame::thing_pick`, `Stage` (`src/render.rs`), `terminal::clean_text` (`src/terminal.rs`), `model::Cpu` with `busy` and `CoreKind` (`src/model.rs`, `src/palette.rs` for core colours). Planned: `src/switchboard.rs`, `src/platform/linux/counters.rs` (B4), the world-space ribbons of S3. The prototype is `docs/mockups/code/switchboard.py` (numpy, cv2, scipy; reference only).

## 9. Variations and open questions

* Draw the real queue: for network lines, show the NAPI poll budget or the softirq `NET_RX` per-CPU rate as a second lamp.
* Replace log-rate lamp brightness by a calibrated saturation scale and say what 8000 irq/s per line is relative to the machine.
* Per-CPU `LOC` rate as a **tick-rate meter**: does a tickless idle CPU show a slower clock?
* Measure how often irqbalance actually moves lines on a real desktop; the swing only shows it if the servicing CPU or effective affinity changes.
* What happens to the board on a machine with 128 CPUs and 4000 vectors (the 128-jack cap and group folding)?
* Show interrupt coalescing: lines whose rate tracks packet rate poorly.

## 10. References

* A. K. Erlang, "Solution of some problems in the theory of probabilities of significance in automatic telephone exchanges", *Elektroteknikeren* 13 (1917); English translation in *Post Office Electrical Engineers' Journal* (1918) (volume and page numbers unverified).
* L. Kleinrock, *Queueing Systems, Volume 1: Theory* (Wiley, 1975), for M/M/N and M/G/N/N.
* A. B. Strowger, US patent for an automatic telephone exchange (1891) (patent number not verified here).
* Linux kernel documentation: `Documentation/core-api/irq/` (interrupt affinity, `irq-domain`), `Documentation/admin-guide/kernel-parameters` for `irqaffinity`; the `show_interrupts` implementation in `kernel/irq/proc.c` and `arch/*/kernel/irq*.c` for the named rows.
* Issue [#48](https://github.com/tfpickard/isotop/issues/48), [#31](https://github.com/tfpickard/isotop/issues/31); `docs/briefs/views-2026-10.md` section 9; [docs/VIEW_CONVENTIONS.md](https://github.com/tfpickard/isotop/blob/main/docs/VIEW_CONVENTIONS.md). See also [Theory-Background-Sampler-and-Demand](Theory-Background-Sampler-and-Demand), [Theory-Picking](Theory-Picking), [Theory-Text-Safety](Theory-Text-Safety).
