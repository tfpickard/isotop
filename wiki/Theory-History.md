# Theory: History

A view that draws the past (Strata's ridges, Ptolemy's epicycles, the flashes of a birth) cannot wait a minute
for the past to accumulate when you switch to it. So isotop keeps **one shared history** of every process's CPU and
memory, recorded on every sample *whichever view is shown*, and lets views read windows of it. This page is that
structure: what a sample holds, the ring and its capacity, the four ways to read it (`window`, `recent`, `series`,
`changes`), how a headless run fills it, what it costs in memory, and a problem it has today with `--sample-ms`
(open issues [#126](https://github.com/tfpickard/isotop/issues/126) and
[#127](https://github.com/tfpickard/isotop/issues/127)) that this page states as it is.

## Two histories, not one

isotop has two things called history and they are unrelated:

| | `App::history` | `Scene::history` (`History`, `history.rs`) |
|---|---|---|
| holds | whole `Snapshot`s | per-process `(cpu, memory)` readings |
| size | `--history`, default 120 snapshots (2 to 3600) | `history::CAPACITY` = 240 samples, fixed |
| purpose | pause, rewind and step: the app draws `history[index]` | views that need the recent past |
| read by | the main loop | views, through `Stage::history` |

The rest of this page is the second. The first is a plain `VecDeque<Snapshot>` in `main.rs`; while paused the main
loop stops sampling and the app draws an older snapshot from it, and the shared `History` ignores any snapshot that
is not newer than its newest, so rewinding cannot corrupt it.

## What a sample holds

```rust
pub struct Reading { pub cpu: f32, pub memory: u64 }
pub struct Sample  { pub elapsed: f64, pub sequence: u64, readings: Vec<(Identity, Reading)> }
```

A `Sample` is one snapshot reduced to what history wants: `elapsed` (seconds on the collector's monotonic clock,
`Snapshot::elapsed`), an absolute `sequence` number (how many samples were recorded before it; it keeps rising when
the ring turns over, so a view can number samples across windows), and the readings **sorted by `Identity`**.
Sorting buys binary search (`Sample::reading`) and a linear merge between two samples (below).

The doc comment says the readings are "the raw readings, not the scene's display smoothing". Be precise about *raw*:
`Reading::cpu` is `Process::cpu`, which in a live run has already been through the model's exponential filter
(time constant 1.5 s, [Model and rates](Theory-Model-and-Rates#the-cpu-filter)); it is raw only relative to the
extra visual easing the scene applies for drawing. In `--demo` there is no filter and the formula value is stored.

## The ring

`History` is a `VecDeque<Sample>`, oldest first, with strictly increasing `elapsed`:

* **`record(snapshot, time)`** appends a sample **unless it is not newer than the newest** (`elapsed` not
  strictly greater, or `NaN`): the scene and a replay may both offer the same snapshot, and a paused app re-offers old
  ones. Then it pops from the front while `len > CAPACITY`. `time` is the *scene* time the sample arrived at, kept as
  `arrived()` so a view that scrolls can glide between samples instead of jumping.
* `Scene::record` calls it on every sample; `Scene::render` calls it again with the snapshot it is drawing, which is
  a no-op in a live run and the fill in a headless or test run that never calls `record`.

`CAPACITY = 240` is a number of **samples**, not seconds. At the default one-second interval it is four minutes.

## Reading it

Every read is relative to an `until`, normally the shown snapshot's `elapsed`, so pause and rewind show the past they
were taken in.

| method | selects | cost |
|---|---|---|
| `window(until, seconds)` | samples with `until − seconds ≤ elapsed ≤ until` | two binary searches (`partition_point`) |
| `recent(until, count)` | the last `count` samples at or before `until` | one binary search |
| `series(id, until, count)` | for one process, its `Reading` in each of those `count` samples, `None` where absent | `count` binary searches; currently unused outside tests |
| `changes(until, seconds)` | births and exits between consecutive samples in a window | a linear merge per adjacent pair |
| `newest()`, `arrived()` | `elapsed` and scene time of the newest sample | O(1) |

**`window` is a duration; `recent` is a count.** The first is how a view should ask: it is correct at any
`--sample-ms`. The second answers "the last N samples" and so means N seconds only at one-second sampling. The
project's own convention (`docs/VIEW_CONVENTIONS.md`) is to express time windows in seconds of `snapshot.elapsed`, not
as sample counts.

### `changes`: births and exits from two sorted lists

Because each sample is sorted by `Identity`, finding who appeared and who left between two consecutive samples is the
merge step of merge sort: walk both lists with a peekable iterator on each; if the old head is smaller, that identity
left; if the new head is smaller, it arrived; if equal, both advance. Each adjacent pair costs $O(n_{\text{old}} +
n_{\text{new}})$. Each `Change` records the identity, the `elapsed` of the first sample it was in (a birth) or the first
it was missing from (an exit), whether it was born, and its first reading (a birth) or its last (an exit). The oldest
sample in the history has nothing before it, so it reports **no births**: a history that starts with a running machine
does not count the machine as newborn.

This is *inference from snapshots*, with the limit that implies: a process that lived and died inside one interval is
never seen, and nothing says why a process left. The exact and detailed alternative is
[`Snapshot::life`](Theory-Life-Events), which exists only while a view demands it. Ptolemy flashes novae and fades from
`changes`; the Necropolis, whose subject is death, uses events.

## Consumers

* **Strata** reads `window(now, 60.0)`: a 60-second window by elapsed time.
* **Ptolemy** reads `recent(until, SERIES)` with `SERIES = 60`, a **count**, and `changes(until, 8.0)` for its flashes.
* **Tesseract** does not use `History`: it needs cumulative CPU seconds at a 0.25 s cadence for 180 s, so its own
  `Worldlines` recorder keeps that (the conventions allow this when the data is not CPU percent and memory).
* **Coop** keeps its own per-process CPU history for the flocks' variation.

## Warm-up: filling the history for a headless run

A live run has its history by the time you press a key, because it has been recording. A headless render
(`--output`, `--benchmark`) has no past, so `warm_up` in `main.rs` makes one:

1. **`replay_history`** (demo, headless runs only): record the demo samples of the `history::SERIES` = 60 whole seconds
   before `--time` (`at = time − back` for `back` from 60 down to 1, skipping `at < 0`), each with the life events since
   `at − 1`. These go straight into `scene.history`, not through `Scene::record`, so no view's own record hook sees
   them. A row that already fills history while it rasterizes (`Phase::History`, Strata) skips this.
2. **The row's own phases**: `Phase::History { seconds }` renders frames at one-second steps; `Phase::Record` records
   one-second steps through `Scene::record`; `Phase::Replay` records at `--sample-ms` steps (`seconds / sample_seconds`
   samples); `Phase::Settle` and `Phase::Journal` render frames to settle motion.

The captured frame's own sample is then recorded by the first `render`. The tests pin the counts: a Coop render has 61
samples in `window(75, 60)` with the replay and 31 without.

Note the spacing: the history replay uses **one-second steps whatever `--sample-ms` is**, while a live run records at
`--sample-ms`. Hold that thought.

## Memory

A reading is an `Identity` (16 bytes with alignment) and a `Reading` (16 bytes): 32 bytes per process per sample, plus
a small `Sample` header. At `CAPACITY = 240`:

| `--processes` / processes live | per sample | full ring |
|---|---|---|
| 192 (demo default) | 6 KB | 1.5 MB |
| 1 000 | 32 KB | 7.7 MB |
| 10 000 | 320 KB | 77 MB |

## A problem with `--sample-ms`, stated honestly

The ring and Ptolemy select by sample *count*; the promises made about them are in *seconds*. They agree only at the
default `--sample-ms 1000`.

![Seconds of history covered by 60 samples and by 240 samples as --sample-ms varies, with the 60-second minute marked](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/history-window-vs-sample-ms.svg)

* **[#127](https://github.com/tfpickard/isotop/issues/127) (retention).** 240 samples is $240\cdot\Delta t$ seconds.
  Below `--sample-ms 250` the ring holds fewer than 60 seconds, so Strata's 60-second `window` is silently cut
  short: at 100 ms the ring covers 24 s. The issue notes this is not a regression (Strata's own ring held 240 samples
  before the shared history existed) and proposes sizing retention by elapsed time, at a memory cost it states
  (about 19 MB for 60 s at 100 ms with 1 000 processes, which matches the table above).
* **[#126](https://github.com/tfpickard/isotop/issues/126) (selection).** Ptolemy's `recent(until, 60)` is the last
  60 *samples*: a minute at one second, 6 s at 100 ms, ten minutes at 10 s. That changes which processes become
  planets and what the discrete Fourier transform sees, while the epicycle period assumes one minute. Because the
  headless replay is at one-second spacing, **a live run and a headless run of Ptolemy disagree at any non-default
  `--sample-ms`**. The proposed fix is to build the window by `Sample::elapsed` (or resample to 60 points over the
  last 60 s).

Both were triaged as non-blocking: the default gives exactly the documented minute. Neither is fixed in the code
described here.

## What a viewer can infer

* A ridge or planet is built from the **last N readings of a filtered CPU figure**, so it inherits the filter's
  delay (about 1.5 s) and the quantisation floor.
* An absent identity in a window means "not in that snapshot": it may not have existed, or may have been an unreadable
  process. History does not distinguish.
* A birth or exit from `changes` is stamped with the sample that saw it, up to one interval after it happened.
* Whatever a view shows about the past at a non-default `--sample-ms` should be read with the two issues above in
  mind.

## Where in the code

* [`history.rs`](https://github.com/tfpickard/isotop/blob/main/src/history.rs): `History`, `Sample`, `Reading`, `Change`,
  `CAPACITY`, `SERIES`.
* [`render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs): `Scene::record`, `Scene::render`,
  `Stage::history`.
* [`main.rs`](https://github.com/tfpickard/isotop/blob/main/src/main.rs): `warm_up`, `replay_history`, `run_phases`,
  `App::history` and `--history`.
* [`strata.rs`](https://github.com/tfpickard/isotop/blob/main/src/strata.rs) and
  [`ptolemy.rs`](https://github.com/tfpickard/isotop/blob/main/src/ptolemy.rs): the readers;
  [`views.rs`](https://github.com/tfpickard/isotop/blob/main/src/views.rs): `Phase`, `Warmup`, `When`.
* Figure script:
  [`docs/figures/history-window-vs-sample-ms.py`](https://github.com/tfpickard/isotop/blob/main/docs/figures/history-window-vs-sample-ms.py).

## References

* `docs/VIEW_CONVENTIONS.md` (the rule that windows are in seconds of `snapshot.elapsed`) and the issues
  [#126](https://github.com/tfpickard/isotop/issues/126), [#127](https://github.com/tfpickard/isotop/issues/127).
* D. E. Knuth, *The Art of Computer Programming*, vol. 3, *Sorting and Searching*: the merge of two sorted lists and
  binary search, which `changes` and `window` are.
