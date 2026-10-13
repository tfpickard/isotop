# View Switchboard

Hardware interrupts drawn as a cord telephone exchange, on Linux. A tall wooden board holds one jack for every interrupt line in `/proc/interrupts`, each with a lamp that flickers at the line's rate; a row of operators sits below it, one for every CPU, performance cores first; and the cord of a line hangs on a true catenary from its jack to the operator whose CPU serviced most of its interrupts in the last interval. When the kernel (or `irqbalance`) moves a line to another CPU, the plug is lifted out, swung across and seated again. A smaller second board to the right carries the softirqs. No process is drawn at all: everything on the screen is a per-CPU interrupt count that the sampler has turned into a rate.

![Switchboard: sixteen operators at a wooden cord switchboard, forty-eight interrupt lines as jacks with amber lamps, coloured cords hanging from each jack to an operator's keyshelf, a clock and two lamps on each operator's rail and a softirq board of ten rows on the right](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/switchboard.webp)

*The README image of the view (`docs/media/switchboard.webp`), rendered from the demo workload: 48 lines on 16 CPUs of a synthetic table (`DemoOptions::exchange`), not a real machine.*

## The system being modelled

**The manual exchange.** In a cord switchboard each subscriber's line ends at a **jack** on the board, with a **line lamp** above it. In a common-battery exchange the lamp lights when the subscriber lifts the handset. An operator seated in front of the board answers by inserting the **answering plug** of a **cord pair** into that jack and throwing a key to speak; she asks for the number and inserts the other plug of the pair, the **calling plug**, into the called party's jack. Each cord has a **supervisory lamp** that tells her when its end hangs up, so she can pull the plugs and free the cord. Operators, cords and trunks to other offices are the scarce resources; jacks are plentiful.

**The interrupt.** A device that needs a CPU asserts an interrupt line. The CPU stops what it was doing, runs a handler for that line and returns. Linux counts, per CPU, how many times each line has fired (`/proc/interrupts`) and how often each deferred-work vector has run (`/proc/softirqs`). The CPUs a line may use are its affinity; the one that really receives it is its effective affinity.

**Why a switchboard is a good picture of it.** The two have the same shape: many cheap lines, few expensive servers and a rule that joins one to the other. The exchange also supplies the right event for the one thing about interrupts that changes slowly enough to watch, the routing: a cord moves from one operator to another.

| Exchange | Machine |
|---|---|
| subscriber line, its jack | an interrupt line (a numbered row of `/proc/interrupts`) |
| off-hook, line lamp lights | the device asserts the line |
| operator at a position | a CPU |
| answering cord plugged in | the line is being serviced by that CPU |
| operator's telephone ringing | a rescheduling inter-processor interrupt (`RES`) |
| trunk to another office | an inter-processor interrupt in general |

The analogy is loose in three ways. A line lamp is a **level** (pending or not); the view's lamp shows a **rate**. A cord in a real exchange joins two parties; here it joins a line to the CPU that handled it, and nothing is ever drawn between two CPUs, because **the sender of an inter-processor interrupt is not recorded** anywhere the kernel exposes. And a real operator serves one call at a time; a CPU services many.

## History

**Wires before telephones.** The first network that beat a rider was optical. Claude Chappe tested a pendulum-and-panel telegraph between Brûlon and Parcé in the Loire on 2 or 3 March 1791 (sources differ on the day, and one gives the distance as about 14 km). The National Convention voted the money for a Paris to Lille line in 1793; it was declared open on 16 July 1794 and its first official message, on 15 August, announced the recapture of Le Quesnoy. By 1844 the French network had 534 towers and more than 5,000 km of line.

The electric telegraph arrived from two directions. William Fothergill Cooke and Charles Wheatstone took out a joint English patent in 1837 (one source says May, another 12 June) for an instrument with five needles. The Science Museum Group dates the first public telegraph service to 1839: 13 miles beside the Great Western Railway, Paddington to West Drayton. The partners fell out, and the credit was later divided between them in an arbitration by Marc Brunel and J. F. Daniell: Cooke for introducing the telegraph in practice, Wheatstone as the man of science. In the United States, Samuel Morse and Alfred Vail's line from Washington to Baltimore carried "What hath God wrought" on 24 May 1844. Whether the code is Morse's or Vail's is disputed, and so is the room in the Capitol the message left from (the Old Supreme Court Chamber, says the Library of Congress; a room in the Senate wing, say Senate historians). The first transatlantic cable, in 1858, carried 98 words from Queen Victoria to President Buchanan, which took about sixteen and a half hours, and was dead within weeks. The cause is disputed: the company's electrician, Wildman Whitehouse, is blamed for raising the voltage as the cable decayed, and an inquiry blamed its manufacture and insulation. The cable that lasted was laid by the *Great Eastern*, which reached Heart's Content, Newfoundland, on 27 July 1866.

**The telephone and the exchange.** The priority of the telephone is the most litigated question here. Alexander Graham Bell's application and Elisha Gray's caveat both reached the Patent Office on 14 February 1876, and Bell's patent, US 174,465, was granted on 7 March. Which came first, and whether Bell's specification was altered or informed by Gray's caveat, was argued for years (the affidavits of Zenas Wilber in 1885 alleged it). The Supreme Court upheld Bell in the Telephone Cases (126 U.S. 1, 1888) and held an alleged interpolation not made out. That is a verdict at law, and it did not end the argument: Philipp Reis (1861) and Antonio Meucci (a caveat of 28 December 1871, renewed in 1882 and 1883, which a federal court found in 1888 did not describe an electric speaking telephone) are still named in print. Credit for the telephone is disputed.

Private pairs of telephones do not scale, because $n$ subscribers want $n(n-1)/2$ pairs; the answer was a switchboard. George W. Coy, a telegraph manager who had heard Bell lecture in New Haven on 27 April 1877, opened the District Telephone Company of New Haven on 28 January 1878 with 21 subscribers at $1.50 a month. His switchboard was, according to one source, made of carriage bolts, handles from teapot lids and bustle wire; the National Park Service says it could join 64 customers, two conversations at a time, with six connections to a call. The first telephone directory, one sheet of 50 names, followed on 21 February. "First exchange" needs its adjective: New Haven is the first commercial exchange, and claims for earlier arrangements elsewhere exist (unverified). The first operators were boys, hired from the telegraph offices, and they were impatient, played jokes and swore. On 1 September 1878 Emma Nutt began work at the Edwin Holmes Telephone Dispatch Company in Boston, and her sister Stella a few hours after her. Popular accounts say Bell hired Emma; others name the company. Her career is given as 33 or 37 years, and the stories of her voice and her memory for the directory are repeated rather than documented.

When an exchange outgrew one board the answer was the **multiple switchboard**, which repeats every subscriber's jack on every operator's section so that any operator can plug any line. Credit is divided: Leroy B. Firman's patent (US 252,576, filed 7 January 1881) and Charles E. Scribner's work, both for Western Electric, and the patents of Sabin were all argued in the suits, and a federal court in 1898 held that Firman had put his invention into practice by March 1879, before Scribner's British application of November 1879. The next step took the operator out. Almon B. Strowger, an undertaker in Kansas City, applied on 12 March 1889 for an automatic exchange and was granted US 447,918 on 10 March 1891 (one history prints 10 May). The first public Strowger exchange opened in La Porte, Indiana, on 3 November 1892, with about 75 subscribers. Why he did it is told as a story in several versions: an operator, in one version the wife of a rival undertaker, was sending his calls elsewhere. One history of the Strowger system tells it "according to the legend", and the only dated account I found, attributed to the local Bell manager Herman Ritterhoff in 1913, says the fault was a metal sign swinging against Strowger's telephone wire. The motive is single-sourced and was written down decades later. The Swedish telephone administration, Televerket, demonstrated a crossbar system in 1919, and the Bell System's No. 1 crossbar was first cut over at Troy Avenue, Brooklyn, in February 1938. The last hand-crank system in the United States, at Bryant Pond, Maine, went over to dial on 11 October 1983, with 425 telephones (UPI).

**The interrupt.** A machine that waits on a slow device can poll it, or let the device ask. Mark Smotherman's survey of the history is the source for the rest of this paragraph. The NBS DYSEAC (1954) is the first machine to use interrupts for input and output, with two program counters. The UNIVAC 1103 is dated 1953 in a 1971 textbook figure, and other authors give the 1103A of 1956, whose interrupt system was built at the NACA Lewis laboratory by L. Richard Turner and Joseph Rawlings and put into operation in February 1956; so "first" depends on which of the two is meant and on whether I/O interrupts are required. The IBM 650 (1954) is credited with the first interrupt masking and the MIT Lincoln Laboratory TX-2 (1957) with multiple priority levels.

Intel's 8259 was introduced in 1976 with the MCS-85 family. The 8259A was in the original IBM PC of 1981, which had one controller and eight interrupt lines; the PC/AT of 1984 added a second, cascaded through IRQ2, and with it the fifteen usable lines that PC software was written for. Multiprocessors needed a controller that could aim a line at a chosen CPU. Intel's APIC began as the discrete 82489DX, for 486 and early Pentium systems, and moved onto the processor with the P54C Pentium (a local APIC in each CPU, I/O APICs in the chipset); Intel's MultiProcessor Specification (pre-release of 27 October 1993, version 1.1 of April 1994) made it the standard. The last step removed the wire. Message-signalled interrupts are in the PCI Local Bus Specification 2.2 (revision dated 18 December 1998): the device performs a memory write instead of pulling a line. MSI-X followed in PCI 3.0 (unverified). That is why one NVMe disk appears as `nvme0q0` to `nvme0q8` on the board, a vector for each queue, and why the view groups them back into one device.

**The mathematics of the board is Danish.** Agner Krarup Erlang (1878 to 1929) joined the Copenhagen Telephone Company in 1908, showed in 1909 that random telephone calls follow the Poisson distribution, and in 1917 published the loss and waiting formulae below. The unit of offered traffic, the erlang, is named for him. The programming language Erlang, written at Ericsson for telephone switches, has its name explained in Joe Armstrong's history of it for HOPL III (2007): some said it meant "Ericsson Language", others that it was named after Agner Krarup Erlang, and the authors "deliberately encouraged this ambiguity".

## Theory

### From wires to messages

A device signals the CPU with an interrupt; the CPU stops, runs a handler and returns. A PC/AT had fifteen shared wires. Modern x86 has a **local APIC** in every CPU and one or more **I/O APICs** that route the legacy wires to a chosen local APIC. PCI devices increasingly use **MSI and MSI-X**: the device performs a memory write to a special address whose data names the vector, so one device can own many vectors with no shared line. The multiqueue hardware and the multi-vector model were built for each other: `nvme0q0` to `nvme0q8`, `eth0-rx-0`, `eth0-tx-1`.

### `/proc/interrupts`

Each numeric row is `IRQ: count-per-CPU... chip hwirq-and-trigger device-names`; an MSI-X vector reads like `147: ... IR-PCI-MSIX ... eth0-rx-2`. The columns follow the `CPUn` header, which leaves offline CPUs out, so a parser maps columns by header, not by position. Besides numeric rows there are **named rows** with a description in place of chip data. On x86 these include `NMI` (non-maskable), `LOC` (local APIC timer), `SPU` (spurious), `PMI`, `IWI`, `RTR`, `RES` (rescheduling IPIs), `CAL` (function-call IPIs), `TLB` (TLB shootdowns), `TRM`, `THR`, `DFR`, `MCE`, `MCP`, `HYP`, `ERR`, `MIS`, `PIN`, `NPI` and `PIW`; `ERR` and `MIS` have a single total rather than a count per CPU. On arm64 there is no `LOC` row, the inter-processor interrupts are `IPI0:` to `IPI6:` rows with a description, and the timer is an ordinary row such as `arch_timer`. Counts are cumulative since boot, so everything the view shows is a rate from a difference.

### Top halves and softirqs

The handler (the "top half") does the minimum and defers the rest as a **softirq**, a tasklet or a workqueue item. The kernel has a fixed list of softirq vectors, all ten on `/proc/softirqs`: `HI`, `TIMER`, `NET_TX`, `NET_RX`, `BLOCK`, `IRQ_POLL`, `TASKLET`, `SCHED`, `HRTIMER`, `RCU`. Network receive uses **NAPI**: the first packet interrupt masks the queue and schedules polling in `NET_RX`, which handles a budget of packets, so a flood becomes a polled stream rather than a storm. When softirqs run too often, a per-CPU `ksoftirqd` thread takes them over so that they compete fairly with user threads.

### Affinity and `irqbalance`

Each numeric IRQ has `/proc/irq/N/smp_affinity` (a bitmask), `smp_affinity_list` and, where the kernel exposes it, `effective_affinity_list`: the CPU or CPUs that really receive it, which can be a subset of the requested mask (on x86 a vector is delivered to one CPU of the mask). The `irqbalance` daemon rewrites affinities from time to time to spread load, and drivers with one vector per queue pin each vector to a CPU. The cord swing is the observable edge of such a rewrite.

### Erlang B: calls cleared

The people who studied exchanges asked: given $N$ operators (or trunks) and calls arriving at random, how often is a caller blocked, or how long does he wait? Calls arrive as a Poisson process of rate $\lambda$; each holds a server for a random time of mean $1/\mu$; there is no queue, and a call that finds all $N$ servers busy is lost. The offered load is $A = \lambda/\mu$ erlangs. The number of busy servers is a birth-death chain; balancing the flow between neighbouring states, $\lambda\, p_{k-1} = k\mu\, p_k$, gives $p_k = p_0 A^k / k!$, and normalising,

$$
B(N, A) = p_N = \frac{A^N / N!}{\sum_{j=0}^{N} A^j / j!}.
$$

Writing $S_n = \sum_{j \le n} A^j/j!$ and $B_n = (A^n/n!)/S_n$ gives $1/B_n = 1 + (n/A)(1/B_{n-1})$, the stable recurrence

$$
B_0 = 1, \qquad B_n = \frac{A\,B_{n-1}}{n + A\,B_{n-1}} .
$$

The result is **insensitive** to the distribution of holding times beyond their mean, which is why it survived a century of changing technology. Computed from the recurrence (`docs/figures/switchboard-erlang.py`) and checked against the closed form ($B(10, 5) = 0.018385$):

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

### Erlang C: calls queue

If blocked calls wait, the system is M/M/$N$ and the probability that an arrival must wait is

$$
C(N, A) = \frac{N\,B(N, A)}{N - A\,(1 - B(N, A))}, \qquad A < N,
$$

with mean waiting time $W_q = C(N,A)/(N\mu - \lambda)$. Pooling pays: at utilisation 0.9 the probability of waiting is 0.90 for one server, 0.79 for four, 0.59 for sixteen and 0.31 for sixty-four, and the mean wait in units of the service time falls from 9.0 to 1.97, 0.37 and 0.049.

![Erlang B and Erlang C curves](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/switchboard-erlang.svg)

### What carries over to interrupts, and what does not

Erlang's results are exact for Poisson arrivals and independent exponential service (for B, any service distribution). Interrupts are neither: packets arrive in bursts, NICs and drivers **coalesce** interrupts, timers are periodic and handler times are heavy-tailed. Interrupts are not blocked and cleared but masked and made pending, except where a finite queue overflows (a full NIC receive ring drops packets, the real "blocked call"). The honest use of Erlang here is as the story of why the view has the shape it has: a few operators (CPUs) pooled over many lines, with a few lines (a busy NIC queue) carrying most of the load, which is why the lamps differ so much in brightness and the cords in thickness. The view makes **no queueing claim**.

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Jack | a numbered row of `/proc/interrupts` (`Snapshot::interrupts`) | the device is the name minus a queue suffix (`device_key`, `[-:_]?(q\|rx-\|tx-\|TxRx-)?\d+$`), so `nvme0q0` to `nvme0q8` are the device `nvme0`; devices stand together, ordered by their lowest IRQ number, with a divider between them; the board has clamp(ceil(sqrt(3 n)), 12, 16) columns for the $n$ lines it first sees and grows rows, never columns; a line keeps its seat while it exists and a freed seat goes to the next newcomer, lowest first (`Pegs`) |
| Lamp brightness | the line's rate: the per-CPU rates of the interval, summed | $\log_{10}(\text{rate}+1)/\log_{10}8000$ (`lamp::JACK_FULL`), clamped to 1; a rate not measured yet (first pass, new row or CPU, a count that went backwards) lights nothing and is never read as 0 |
| Lamp flicker | the same rate | event $k$ flashes at $(k + \text{jitter}(\text{irq},k))/f$, $f = \min(\text{rate}, 0.9\,\text{fps})$ (`Stage::fps`), a flash decaying with a 50 ms time constant; the inspector shows the true rate and the cap |
| Cord's operator | the per-CPU rates of the line, not the cumulative counts | the seat whose CPUs have the largest summed rate, the plugged seat winning a tie; a line with no events keeps its cord; a line never seen serviced has none, unless its effective affinity is a single CPU |
| Cord swing | the line's `effective_affinity_list`, and its servicing CPU | the cord swings the moment the effective affinity changes, and after a new servicing seat has persisted for `PERSISTENCE` = 2 samples; the plug lifts for 0.35 s, swings on an arced smoothstep for 1.05 s and seats for 0.25 s (`MOVE` 1.65 s); the colour crossfades and the lamp flutters while it is out |
| Cord shape | the line's number | a true catenary of 24 segments (`cord::catenary`) of length chord times a slack of 1.02 to 1.07 hashed from the IRQ number (`cord::slack`), hung again only when an end moves |
| Cord width | the line's rate | $(0.6 + 0.5\log_{10}(\text{rate}+1))$ clamped to 0.6 to 3.0 board units (`cord::width`), times `CORD_WEIGHT` 1.6 |
| Cord colour | the operator's core kind and number | a warm table for performance cores, a cool one for efficiency cores and unknown kinds (`cord::operator_color`) |
| Operator | one CPU | performance cores first, then efficiency, then unknown, by CPU number within each; at most 64 seats (`SEATS`), above that each covers $\lceil n/64\rceil$ CPUs and the legend says so; head and arm motion scale with `Cpu.busy` |
| Operator's clock | `LOC` (arm64: the `arch_timer` row, `kernel::timer_row`) | one turn per 6000 counts, interpolated between samples from the rate; without a timer row the clocks stop and the legend says so |
| Operator's telephone | `RES` (arm64: the `Rescheduling` row) | rattles while the seat's rate is at least 1 a second |
| Green lamp | `CAL` plus `TLB` (arm64: `Function call`) | $\log_{10}(\text{rate}+1)/\log_{10}1000$ (`CALLS_FULL`), clamped to 1 |
| Red lamp | `NMI` | lit for 2 s (`NMI_LIT`) by an increase of the count |
| Softirq board | `/proc/softirqs`, ten rows | one lamp per operator per softirq, $\log_{10}(\text{rate}+1)/\log_{10}5000$ (`lamp::SOFTIRQ_FULL`), dark under half an event a second; a rate not known yet is a darker lamp than a measured zero |
| Wood, chairs, figures, cornice, brass, lighting | none | **decorative** |

The other named rows are not drawn. The header (`ViewModule::inspector_line`) reads like ` 48 lines | 41,203 irq/s | RES 2,114/s | 16 operators: 8 P, 8 E`, and ` N lines on M jacks` when lines are folded; the totals are the demo's.

## How isotop realises it

**Module and registry row.** `src/switchboard.rs` holds `Switchboard` (a `#[derive(Default)]` struct implementing `views::ViewModule`), with `lines`, `cord`, `lamp`, `scenery` and `figures` as submodules under `src/switchboard/`. The row at the end of `views::REGISTRY` sets `Backdrop::Dusk`, `pitch: Some(0.45)`, `links: LinkRule::DrawnByView` (there are no sockets to link), `tour: false`, `haze` and `flash` off, `sources` of `Source::Interrupts`, `Source::Softirqs` and `Source::IrqAffinity`, `demo` of `DemoOptions { exchange: true, .. }` and a `contract` with `draws_processes: false`. It has no keys and no digit. `draw` returns 0; the header is the view's own, returned from `inspector_line` (the row's `header` is counts only). There is no plugin system: the view is compiled in.

**Data.** The three sources are read on the background thread every `model::BACKGROUND_PERIOD` (2 s) and only while the shown view declares them. `model/kernel.rs` (`KernelScan`) turns the counts into per-CPU rates from the change since the previous pass: `None` on a first pass, a new row or CPU and a count that went backwards (never 0, never negative), with the interval floored at 0.1 s. Each collector sample (`--sample-ms`, 1 s by default) carries the latest reading, so a rate is a 2-second average and consecutive samples can carry the same one. `PERSISTENCE` counts samples, not background passes, so on my reading of the code a change of servicing CPU shows one to two seconds after a reading carries it, at the defaults; a change of effective affinity does not wait. The view reads only the `Snapshot`, so a paused or rewound frame is drawn as it was, with no swing.

**Lines, seats and cords** (`src/switchboard/lines.rs`). `Reading::new` reads a snapshot into numbered lines and the named rows; `Lines::take_in` seats the lines with `Pegs` (keyed by IRQ number as a `u64`, never an `Identity`), picks each cord's target (`servicing_seat`, `affinity_seat`) and starts a swing. The board is capped at `CAP` = 128 jacks. Above that, lines whose count never changed since the view began watching go first, then each device of more than `FOLD_ABOVE` = 8 lines folds into one jack that sums its rates and is named like `nvme0q* (33)`; the legend says how many lines were folded. A frame after a gap (another view was shown, or time went back) puts every cord where the sample says and swings none.

**Cords and lamps.** `cord::catenary` solves $2k\sinh(h/2k) = \sqrt{L^2 - v^2}$ for the catenary parameter $k$, by 80 steps of bisection on $\ln k$, for a cord of length $L$ over a span $h$ and a rise $v$. For a level cord of slack $s$ the sag is close to $h\sqrt{3(s-1)/8}$: $0.087h$, $0.132h$ and $0.166h$ at $s$ = 1.02, 1.045 and 1.07 (computed). `Switchboard::hang` cuts the slack by a factor of 0.6, at most 12 times, until the cord hangs no more than 3 board units below its lower end, and keeps the points until an end moves. Flicker is stateless and repeatable from the line and the time (`lamp::events`), so the same time gives the same lamps; the operators' motion steps on `simulation::FixedStep` at 1/60 s. A cord is a 24-segment `Frame::ribbon`, with a dark edge underneath when there are at most 64 jacks.

**Drawing, parity and budget.** The view draws only triangles, ribbons and puffs, never `Item::Line`, `Item::Beam` or `Item::Glow`, so that the CPU and GPU backends rasterize alike; the slight yaw of the board (`scenery::SHEAR`) is built into its geometry because the camera has no shear. A test asserts fewer than 11,500 triangles for the demo and glows within 12 px at 1080 lines. I did not compare a GPU render with the CPU one here (unverified).

**Picking, text and the demo.** A jack, its lamp, its socket and its cord pick the line (a `Thing` of kind `LINE`, through `Frame::thing_pick`); the card (`ViewModule::card`) gives name, IRQ and chip, rate and the lamp's flicker cap, the servicing CPU, effective affinity, the last move and the three CPUs with the largest shares. Operators and softirq lamps are not pickable. All device text goes through `terminal::clean_text`; labels go through `stage.places`. The demo table (`src/model/exchange.rs`) has 48 lines on 16 CPUs at 41,203 events a second in all, `eth0-rx-2` moving between cpu3 and cpu9 at 10.2 s and every 30 s after, a non-maskable interrupt on cpu0 every 45 s from 10.7 s, and a rescheduling rate that follows how busy each CPU is. The row replays three seconds before `--time` (`Phase::Play`, a frame every 0.1 s), so at `--time 12` the moved cord is a second into its swing and the red lamp is lit.

**Platforms.** Linux only; all the sources are unprivileged. A missing one is named in the legend: without `/proc/interrupts` the board stays dark and the header says so; without affinities only a change of servicing CPU swings a cord; without `/proc/softirqs` the softirq board is dark; without a timer row the clocks stop. macOS exposes no unprivileged per-IRQ or per-CPU counters (`powermetrics` needs root; IOReport is private), so `switchboard::available` is `cfg!(target_os = "linux")`: Tab and the number keys pass over the view, `--verbose` prints `skipping view switchboard: not supported on this platform`, and there is no "not supported" screen. The module still compiles there.

## Reading the picture

* **Measured:** which lines exist and their rates (2-second averages), which CPU serviced most of each, the effective affinity and its changes, the per-CPU timer, reschedule, call and TLB counts, and the softirq counts per CPU.
* **Derived:** lamp brightness (the log of a rate), cord thickness, the swing (a state machine over two measured quantities), the clock hand (a count converted to an angle).
* **Decorative:** wood, chairs, operators' bodies, the cord slack and the flicker's pattern. A lamp flickers at $\min(\text{rate}, 0.9\,\text{fps})$ events a second **by display convention** and its phase is a hash, so its rhythm is not the interrupt arrival process. A flicker at $f$ sampled at fps appears at $\lvert f - n\cdot\text{fps}\rvert$, so at $f = 0.9\,\text{fps}$ the beat is slow and at $f = \text{fps}$ the pattern would freeze; the cap and the jitter avoid that. That is my inference about the choice.
* **Cannot infer:** which CPU *sent* an inter-processor interrupt; which process caused an interrupt (no attribution); a line's individual interrupt times; any queueing or latency.
* **Misreadings to avoid:** a cord shows the **plurality** servicing CPU, not the affinity mask, so a line spread evenly over eight CPUs shows one arbitrary cord; a cord that does not move may be a quiet line, not a pinned one; a lamp's brightness is a logarithm, so two lamps one step apart differ by far more than a step.

## Try it

Commands run with the release binary on Linux (CPU renderer), repository root:

```sh
./target/release/isotop --list-views
./target/release/isotop --demo --view switchboard --time 12 --width 1600 --output /tmp/switchboard.png --renderer cpu
head -3 /proc/interrupts; cat /proc/softirqs; cat /proc/irq/*/effective_affinity_list | sort | uniq -c
```

The first lists twenty-one views, `switchboard` last (the order of `views::REGISTRY`). The second wrote `Wrote /tmp/switchboard.png (1600x900, 0 visible)`: no processes are drawn, hence none visible. The third shows the tables the view reads; on the machine I ran it on, which has two CPUs, they are short. Live: `./target/release/isotop --view switchboard`, or Tab and the `v` picker; `l` hides the labels, and selecting a jack or a cord opens its card. `python3 docs/figures/switchboard-erlang.py` regenerates the queueing figure.

## Where in the code

| Item | Path and symbol |
|---|---|
| The view | [src/switchboard.rs](https://github.com/tfpickard/isotop/blob/main/src/switchboard.rs): `Switchboard`, `available`, `Switchboard::hang`, `draw_lines`, `draw_cord`, `draw_operators`, `draw_softirqs` |
| Lines, seats, swings | [src/switchboard/lines.rs](https://github.com/tfpickard/isotop/blob/main/src/switchboard/lines.rs): `Reading`, `Lines`, `Pegs`, `Roster`, `device_key`, `CAP`, `PERSISTENCE` |
| Cords | [src/switchboard/cord.rs](https://github.com/tfpickard/isotop/blob/main/src/switchboard/cord.rs): `catenary`, `slack`, `width`, `operator_color`, `plug_at`, `MOVE` |
| Lamps | [src/switchboard/lamp.rs](https://github.com/tfpickard/isotop/blob/main/src/switchboard/lamp.rs): `brightness`, `flicker_hz`, `events`, `jack`, `softirq` |
| The room and the people | [src/switchboard/scenery.rs](https://github.com/tfpickard/isotop/blob/main/src/switchboard/scenery.rs) (`Plan`, `Pen`) and [src/switchboard/figures.rs](https://github.com/tfpickard/isotop/blob/main/src/switchboard/figures.rs) |
| Registry row | [src/views.rs](https://github.com/tfpickard/isotop/blob/main/src/views.rs): `REGISTRY` row `switchboard` |
| Sources and rates | [src/model/kernel.rs](https://github.com/tfpickard/isotop/blob/main/src/model/kernel.rs) (`KernelScan`, `timer_row`), [src/platform/linux/counters.rs](https://github.com/tfpickard/isotop/blob/main/src/platform/linux/counters.rs), `platform::Source` in [src/platform/mod.rs](https://github.com/tfpickard/isotop/blob/main/src/platform/mod.rs) |
| Demo | [src/model/exchange.rs](https://github.com/tfpickard/isotop/blob/main/src/model/exchange.rs): `DemoOptions::exchange` |
| Shared helpers | `status::missing_note` and `sources_note`, `simulation::FixedStep`, `Frame::ribbon` in [src/render/strokes.rs](https://github.com/tfpickard/isotop/blob/main/src/render/strokes.rs), `terminal::clean_text` |
| Prototype (reference only) | [docs/mockups/code/switchboard.py](https://github.com/tfpickard/isotop/blob/main/docs/mockups/code/switchboard.py) |

## Variations and open questions

* The design in the issue differs from the code in five places. A decreasing counter was to read 0 and is unknown (`None`) in the code; the board was to take about 4k triangles and the test allows under 11,500; the design called for a local equivalent of `layout::Seats` and the code has `Pegs`; the demo move was at 11.2 s where the code moves the line at 10.2 s, so that the frame at 12 s is a second into the swing; and the design derived a change of servicing CPU showing after about 4 s, where the code counts samples and gives one to two seconds at the defaults.
* Draw the real queue: for network lines, show the NAPI poll budget or the `NET_RX` rate per CPU as a second lamp.
* Replace log-rate brightness by a calibrated saturation scale and say what 8000 events a second per line is relative to the machine.
* Per-CPU `LOC` rate as a tick-rate meter: does a tickless idle CPU show a slower clock?
* Measure how often `irqbalance` actually moves lines on a real desktop; the swing shows it only if the servicing CPU or effective affinity changes.
* A machine with 128 CPUs and 4000 vectors exercises the 128-jack cap and the folding; it was not run here (unverified).
* Offered load. One erlang is one server continuously busy, so the operators carry between them the sum of their `Cpu.busy` fractions in erlangs; sixteen CPUs at 50 % carry eight.
* Show interrupt coalescing: lines whose rate tracks the packet rate poorly.

## References

* A. K. Erlang, "Sandsynlighedsregning og Telefonsamtaler" (The theory of probabilities and telephone conversations), *Nyt Tidsskrift for Matematik* B 20 (1909) 33-39; "Løsning af nogle Problemer fra Sandsynlighedsregningen af Betydning for de automatiske Telefoncentraler" (Solution of some problems in the theory of probabilities of significance in automatic telephone exchanges), *Elektroteknikeren* 13 (1917) 5-13; an English translation appeared in the *Post Office Electrical Engineers' Journal* (1918) (volume and pages unverified).
* L. Kleinrock, *Queueing Systems, Volume 1: Theory* (Wiley, 1975), for M/M/N and M/G/N/N.
* *Telephone Cases*, 126 U.S. 1 (1888); US patents 174,465 (Bell, 1876), 447,918 (Strowger, 1891), 252,576 (Firman, 1882) and 262,701 (Scribner, 1882).
* M. Smotherman, "History and Overview of Interrupts and Interrupt Systems" (web page, Clemson University), which cites A. L. Leiner, "System Specifications for the DYSEAC", *Journal of the ACM* 1(2) (1954) 57-81, and L. R. Turner and J. H. Rawlings, *IRE Transactions on Electronic Computers* EC-7(2) (1958) 141-149.
* J. Armstrong, "A history of Erlang", *Proceedings of the Third ACM SIGPLAN Conference on History of Programming Languages* (HOPL III, 2007).
* Intel, *MultiProcessor Specification* (version 1.4); *PCI Local Bus Specification*, revision 2.2 (1998); IBM, *Personal Computer AT Technical Reference* (March 1984).
* National Park Service, "Site of the First Telephone Exchange"; Science Museum Group, "Cooke and Wheatstone five-needle telegraph"; G. Cookson, "The Atlantic cable", *History Today*; IET Archives, "The Transatlantic Telegraph Cables 1865-1866"; J. Fleming, "Maine town records nation's last crank call", UPI, 11 October 1983.
* Linux kernel documentation, `Documentation/core-api/irq/`, and the `show_interrupts` implementation in `kernel/irq/proc.c` and `arch/*/kernel/irq*.c` for the named rows.
* Issues [#48](https://github.com/tfpickard/isotop/issues/48) and [#31](https://github.com/tfpickard/isotop/issues/31); [docs/VIEW_CONVENTIONS.md](https://github.com/tfpickard/isotop/blob/main/docs/VIEW_CONVENTIONS.md). See also [Theory Background Sampler and Demand](Theory-Background-Sampler-and-Demand), [Theory Picking](Theory-Picking), [Theory Text Safety](Theory-Text-Safety), [Theory GPU and Parity](Theory-GPU-and-Parity).
