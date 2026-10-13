# Guide: Writing a view

There is no plugin system. A view in isotop is one Rust module and one row of a table, compiled into the binary; "writing a plugin" means writing that module, adding that row and sending a pull request. This page is the recipe, and it is a complete one: it builds a new view from nothing, step by step, and every listing in it is code that was applied to a clean checkout of `main` (commit a34733c, which includes the removal of `Hooks`) and built, with clippy clean and its tests passing; [The gates](#the-gates) says exactly which gates were run on that checkout. The view is called `lissajous`. It is a toy in size (one file, about 540 lines before its tests and just over 800 with them) and deliberately not a toy in method: it reads the shared history, seats its plates so that they never move, steps a spring on a fixed clock, hands the app its picks, its labels and its tour, keeps a record hook, and is covered by the same harness as the eighteen views that ship. The last section is speculation about what a real plugin interface could look like, clearly labelled as such.

Read [Theory: Stable Layout](Theory-Stable-Layout), [Theory: History](Theory-History) and [Theory: Simulation Toolkit](Theory-Simulation-Toolkit) for the ideas the code leans on, and `AGENTS.md` and `docs/VIEW_CONVENTIONS.md` in the repository for the rules a pull request is held to. Where this page and those disagree, they win; where they and the code disagree, the code wins.

## History

Programs that draw while someone else is listening or idling have offered outsiders a way in for a long time, and almost every offer has been the same shape: a host that owns the window and the clock, and a small unit that is handed the time (or the sound) and asked for one frame.

* **Screen savers.** Berkeley Systems released After Dark for the Macintosh in 1989 and for Windows in 1991; Wikipedia credits Jack Eastman and Patrick Beard, and its Flying Toasters are the best-known module. A Windows screen saver is an ordinary executable renamed `.scr` that the system starts with a switch for full screen, configure or preview (`/s`, `/c`, `/p`). The contract is a command line, which is as narrow as a contract can be.
* **Light synthesizers.** Jeff Minter's Psychedelia (Llamasoft, 1984, Commodore 64) and Trip-a-Tron (1988, Atari ST) were not plug-ins; they were instruments that drew coloured light from a joystick. His Virtual Light Machine later shipped on the Atari Jaguar CD add-on. They matter here because they made the picture the product and the music optional.
* **Winamp.** Visualization plug-ins were DLLs the player loaded; the player passed each one the current waveform and spectrum and asked it to render (the exact header is from memory, unverified). Ryan Geiss's Geiss plug-in dates from around 1998. Advanced Visualization Studio, designed by Winamp's author Justin Frankel, has an initial release dated March 2000 in Wikipedia; one source has it first shipping with Winamp 2.61, so the date varies by source. MilkDrop followed, released by Geiss on 5 November 2001 according to Wikipedia, built for GPU hardware; its source was released under a BSD licence in May 2005, after which Geoff Potter continued it. AVS made the extension a language: a preset is a list of effects with expressions in them, and its SuperScope draws a curve from an expression, which is to say an oscilloscope.
* **The demoscene** took the opposite position. A production is one executable, often under a fixed size such as 64 kilobytes, and there is no host to be polite to.

The pattern across these is that the contract tends to shrink to what the host can afford to trust: a command line, a pair of arrays, a preset. Whether to load strangers' code at all is a separate question; the speculative section at the end of this page takes it up and, on this project's terms, answers no.

isotop's own contract is short enough to date from the repository (read-only `git log`, commit dates as recorded). The first commit is of 2026-10-07. The registry (`views::REGISTRY`, `ViewInfo`), the `ViewModule` trait, the `Stage` outputs, `FixedStep` and the harness that runs every row all arrived together in PR 120 on 2026-10-11, with a `Hooks` enum whose `BuiltIn` arm kept the eleven views that predated it in the scene's own match arms. PR 143 on 2026-10-12 removed `Hooks`, moved the built-in views (City, Orbit, Ripple, Flow, Matrix and Coop among them) onto `ViewModule`, and made a registry row `ViewInfo::new(name, title, backdrop, build)`. This page was written between those two, which is why its first version did not compile on `main`. By commit date, the views the harness records as predating it are at most four days older than it.

## What is true: views are compiled in

The contract is the trait `views::ViewModule` and the row type `views::ViewInfo`. Every view in the registry, the original City, Orbit, Ripple and Flow included, is a `ViewModule` built by its row's `build`; there is no other path (the `Hooks` enum that once offered one was removed in PR 143). Everything else follows from them.

```rust
// src/views.rs
pub trait ViewModule {
    /// Draws the view onto the stage and returns how many of `processes` it showed; the scene
    /// counts the rest as collapsed. Fill `stage.positions` for every drawn process, and
    /// `stage.bounds` when the content moves by itself. Step by `stage.dt`, and tell the app
    /// anything else only through the other `Stage` outputs, such as `tour` and `popup`.
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize;

    /// Takes in each new sample, whichever view is shown, for history the view needs when it
    /// opens.
    fn record(&mut self, _snapshot: &Snapshot, _time: f32) {}

    /// The third status line while no process is inspected. It starts with a space.
    fn legend(&self, snapshot: &Snapshot) -> String;

    /// Handles a key while the view is shown and no search is open; see `ViewKey` for what the
    /// return value means.
    fn key(&mut self, _key: ViewKey) -> bool {
        false
    }

    /// A line that replaces the inspector line, the second status line, and hides the inspector
    /// popup, for a view that draws no processes. It starts with a space.
    fn inspector_line(&self) -> Option<String> {
        None
    }

    /// The inspector card of `thing`, one of the things the view drew with `Frame::thing_pick`,
    /// or None once it no longer shows it. Called only for the selected or hovered thing, so
    /// build the card here from the view's own state, never one per thing per frame. The
    /// snapshot is the shown sample, for figures the view does not keep itself.
    fn card(&self, _thing: Thing, _snapshot: &Snapshot) -> Option<Card> {
        None
    }

    /// The view's state as `Any`, for `Scene::module`: a module's own tests read what its legend
    /// and `Stage` outputs do not tell. None, the default, shows nothing.
    #[cfg(test)]
    fn state(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Where the view harness looks for each drawn process instead of where it is drawn, for a
    /// view whose bodies move by themselves: the place each one keeps, read after a frame. None,
    /// the default, checks `Scene::positions`.
    #[cfg(test)]
    fn anchors(&self) -> Option<HashMap<Identity, Point>> {
        None
    }
}
```

| Hook | Called by | When |
|---|---|---|
| `draw` | `Scene::render` through `draw_view`, with a `Stage` built for the frame | Every frame the view is shown. Returns how many processes it showed; the rest count as collapsed |
| `record` | `Scene::record`, for every registered row | Every new sample, **whichever view is shown**, and in headless warm-ups (`Phase::Record`, `Phase::Replay`) |
| `legend` | `Scene::legend` | The third status line, while no process is inspected. It starts with a space |
| `key` | `Scene::key`, from `App::command` | A key press while the view is shown and no search or picker is open: `ViewKey::Local(c)` for `c` in `views::LOCAL_KEYS` (`bijkmopuxyz,.;`), `ViewKey::Reset` for Home and `r` (the global reset happens as well), `ViewKey::Escape` (returning true consumes the press) |
| `inspector_line`, `card` | the status line and the inspector popup | A view that draws no processes, or draws things that are not processes (`Frame::thing_pick`); this tutorial uses neither |
| `state` | `Scene::module`, in tests only | A module's own tests read what its legend and `Stage` outputs do not tell; this tutorial's tests read through the legend instead, so it omits it |
| `anchors` | the test harness only | A view whose bodies move by themselves returns the places they keep; this tutorial's plates do not move, so it omits it |

What a view may read is `Stage`, the process slice and `&Snapshot`, and nothing else. It never reads `/proc`, opens a file or a socket, or touches the terminal. What it may write back is `Stage` outputs and the display list it records into `stage.frame`:

| `Stage` field | Direction | Meaning |
|---|---|---|
| `frame`, `camera` | draw into, read | The display list being recorded and the camera it is projected with |
| `selected`, `thing`, `toured`, `focus`, `limit` | read | The selected process (and thing), the process the tour is visiting, the focused subtree's root, `--limit` |
| `time`, `dt` | read | Seconds on the animation clock; the step policy: 0 on the first frame, while paused and after a rewind, otherwise the time since the last frame that moved forward, at most `render::MAX_DT` (0.5 s) |
| `history` | read | Every process's CPU and memory over the last samples (`history::History`), filled by the scene on every sample whichever view is shown |
| `positions` | write | Where each drawn process is: picking anchors, labels, the tour and the exit flash all read it |
| `bounds` | write | A fixed envelope for the camera fit when the content moves by itself |
| `places` | write | World-anchored text labels |
| `notes`, `popup`, `callout` | write | Extra inspector lines after the sockets; extra lines right after the process's own figures; extra lines in the tour's callout |
| `tour` | write | `TourTarget { id, position, weight }`: processes the tour visits, heaviest first |

A view is made known to the app by one row of `views::REGISTRY`, built with `ViewInfo::new(name, title, backdrop, build)`, where `build` is a `views::Build` (`fn() -> Box<dyn ViewModule>`, usually a closure), and a struct update for the fields that differ from the defaults. Tab, Shift+Tab, the `v` picker, `--view`, `--views`, `--list-views` and `--verbose` all follow from the table. The first ten rows are bound to the digits and must never be reordered; a new view goes at the end.

## The view we build

### The system being modelled

An oscilloscope in XY mode plots one signal against another instead of against time. Feed it two sinusoids of the same frequency and it draws an ellipse; of frequencies in a small integer ratio, a Lissajous figure (after Jules Antoine Lissajous, who studied such figures optically in the 1850s). The shape tells the phase between the signals without a clock.

A process's CPU is one signal, not two. The standard trick for one signal is delay-coordinate embedding: plot $x(t)$ against $x(t-\tau)$. Takens (1981) showed that, for the right class of deterministic systems and generic observations, such a plot of a scalar series reconstructs the topology of the system's state space; for a measured CPU series the plot is a phase portrait, not a proof, but it is a good one.

### Theory: what the figure is

For a pure sinusoid of period $T$ and amplitude $a$, let $x = a\sin\theta$ and $y = a\sin(\theta-\delta)$ with $\delta = 2\pi\tau/T$. Then

$$
x^2 - 2xy\cos\delta + y^2 = a^2\sin^2\delta ,
$$

because $y = a(\sin\theta\cos\delta - \cos\theta\sin\delta)$ and the cross terms cancel when it is expanded. This is an ellipse whose axes lie along the two diagonals of the plot. Its semi-axes are $a\sqrt{1+\cos\delta}$ along $x=y$ and $a\sqrt{1-\cos\delta}$ along $x=-y$. At $\delta\to 0$ the ellipse collapses onto the diagonal: a process that varies slowly compared with the delay barely leaves it. At $\delta=\pi/2$ it is a circle, and at $\delta=\pi$ it collapses onto the other diagonal. A steady process is a point on the diagonal, wherever its CPU puts it; a periodic one circles the diagonal; and a signal with harmonics draws a loop with the extra lobes.

The demo makes the numbers concrete. Its process $i$ uses CPU proportional to $\left(\tfrac12+\tfrac12\sin(0.55\,t+1.71\,i)\right)^4$, which has period $T=2\pi/0.55\approx 11.4$ s. The delays the view offers are 2, 4, 8 and 1 s. For the fundamental of that signal, the semi-axes (in units of $a$) are:

| Delay $\tau$ | $\delta = 2\pi\tau/T$ | along $x=y$: $\sqrt{1+\cos\delta}$ | along $x=-y$: $\sqrt{1-\cos\delta}$ |
|---|---|---|---|
| 2 s | 1.10 rad | 1.206 | 0.739 |
| 4 s | 2.20 rad | 0.641 | 1.260 |
| 8 s | 4.40 rad | 0.832 | 1.143 |
| 1 s | 0.55 rad | 1.361 | 0.384 |

So the default (2 s) is a fairly open loop, longer along the diagonal $x=y$; 4 s flips it to the other diagonal; 8 s is a rounder loop, still longer along $x=-y$; 1 s is nearly the diagonal itself. (The demo's real signal has strong harmonics, so its loops are rounded rectangles, not ellipses; the ellipse is the fundamental's contribution. The test `a_delayed_sinusoid_draws_the_ellipse_of_a_lissajous_figure` checks the equation against the samples of a pure sinusoid.)

### Mapping: every element is a measurement or declared decorative

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Plate | One per drawn process | A square of side 3.0 world units, turned so its axes run across and up the screen under the default camera |
| Place of a plate | Process group (`Process::group`), seat | The group picks a disc (`Discs`); the seat, the lowest free number in the group, kept for life, picks a point on a golden-angle spiral of spacing 2.9 in it |
| Horizontal axis | CPU of the shown sample (shared history) | $c/(c+60)$, so one core (100 %) is at 0.625 of the way across and no CPU leaves the plate |
| Vertical axis | CPU one delay earlier (shared history) | The same transform of the CPU at `elapsed - delay`, linear between samples, held at the process's first reading before it existed |
| Figure | The last 30 s of the two axes | A point every 0.5 s ending at the shown sample, whatever `--sample-ms` is; only for the 64 processes whose CPU swung most, and only if it swung by 1 percentage point or more; older stretches dimmer and thinner |
| Diagonal, gridlines | Steady CPU; one core | The line $x=y$; lines at 100 % |
| Bead colour | Kind and state | `palette::kind_color` |
| Bead size | Memory (RSS; the physical footprint on macOS) | $0.07 + 0.03\log_2(\mathrm{MiB})$ world units, at most 0.26 |
| Bead glow | CPU | $c/(c+60)$, rising at once and falling with a time constant of 1.5 s |
| Peak tick | The highest CPU the process has had since the view started recording it | A short segment across the diagonal at that CPU, as of the shown sample, so rewind shows the past |
| Group label | Process group | The ten groups with most drawn processes, by `label_name` |

**Decorative:** the floor tint of a plate, the graticule frame, the figure's brightness and width gradient, the glow's radius, and the bead's position itself: it is not the measurement but a damped spring (natural frequency 1.2 Hz, damping ratio 0.55) pulled towards it, so it overshoots a little. The inspector shows the exact values.

## Step by step

Every listing below is a piece of `src/lissajous.rs`, in file order; together they are the whole file apart from the tests, which come at the end.

### Step 1: the module, its constants and its imports

A view starts with a doc comment that says what it shows, and constants that say what its numbers are. Constants are named, with units and the reason in a doc comment, because the README's mapping table must quote them and the wiki page of the view will too.

```rust
// src/lissajous.rs
//! Lissajous: every process draws its own CPU as an XY figure, the way an oscilloscope in XY mode
//! draws two signals against each other. The horizontal axis is the CPU now and the vertical axis
//! the CPU one delay ago, so a steady process sits on the diagonal and a periodic one circles it.
//! A pure sinusoid of period T, delayed by tau, draws the ellipse of a Lissajous figure with equal
//! frequencies and a phase of 2 pi tau / T; any other signal draws its delay-coordinate portrait.
//! Each process has a plate, seated by its group so that it never moves, and a bead on a damped
//! spring that rides the end of its figure.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::f32::consts::FRAC_1_SQRT_2;

use crate::history::History;
use crate::layout::{Discs, Seats, spin, spiral};
use crate::model::{Identity, Process, Snapshot, bounded, top_n};
use crate::palette::{kind_color, tint};
use crate::render::{Camera, Color, Frame, Point, Stage, TourTarget, label_name};
use crate::simulation::{FixedStep, smoothing};
use crate::views::{ViewKey, ViewModule};

/// Seconds of figure drawn, ending at the shown sample, and the seconds between its points
/// whatever `--sample-ms` is.
const TRACE: f64 = 30.0;
const POINT_EVERY: f64 = 0.5;
/// The delays `m` steps through, in seconds; the first is the default.
const DELAYS: [f64; 4] = [2.0, 4.0, 8.0, 1.0];
/// `bounded` maps a CPU c, in percent, to c / (c + KNEE): 0 to 0, one core to 0.625 of the way
/// across a plate and no CPU past 1, so nothing leaves the plate.
const KNEE: f32 = 60.0;
/// At most this many processes earn a figure, the ones whose CPU swung most, and only if they
/// swung by at least `BUSY` percent. The rest are a bead, which still says where they are.
const FIGURES: usize = 64;
const BUSY: f32 = 1.0;

/// A plate is a square of this side in world units, turned so that its axes run across and up
/// the screen under the default camera.
const SIDE: f32 = 3.0;
const ACROSS: [f32; 2] = [FRAC_1_SQRT_2, -FRAC_1_SQRT_2];
const UP: [f32; 2] = [-FRAC_1_SQRT_2, -FRAC_1_SQRT_2];
/// The share of a plate its axes use, so that a process at rest sits inside its frame.
const PLOT: f32 = 0.84;
/// Seats lie on a golden-angle spiral with this spacing; neighbours are at least 1.6 times as
/// far apart, clear of a plate's diagonal.
const SEAT_SPACING: f32 = 2.9;
const SEAT_OFFSET: f32 = 1.0;
const NAMED: usize = 10;

/// The bead is a damped spring pulled towards the end of its figure: a natural frequency of
/// 1.2 Hz and a damping ratio of 0.55, so it overshoots a little. It is stepped on a fixed clock.
const OMEGA: f32 = 1.2 * std::f32::consts::TAU;
const ZETA: f32 = 0.55;
const STEP: f32 = 0.02;
const MAX_STEPS: usize = 10;
const MAX_GAP: f32 = 0.2;
/// The glow rises with the CPU at once and falls with this time constant, like a phosphor.
const PERSISTENCE: f32 = 1.5;

const FLOOR: Color = [8, 22, 28];
const GRATICULE: Color = [28, 62, 66];
const SELECTED: Color = [255, 235, 171];
const PEAK: Color = [226, 240, 255];
```

Three details already matter. `bounded` (from `model`) is the saturating transform $v/(v+k)$ that every view uses for "how busy" without letting a hot process leave its slot. `top_n` is the shared budgeted selection (largest key first, ties by `Identity`, NaN last), and the budget it describes is the reason a view never sorts the whole machine itself. And `Frame::hairline`, not `Frame::line`, draws every stroke: a new view uses thin triangles because both rasterizers fill them alike (the CPU/GPU parity rule), where one-pixel lines differ between the diamond-exit rule of hardware and the CPU's DDA.

### Step 2: time. `stage.dt` for the glow, `FixedStep` for the spring

Two things in the view change with time, and they need two different tools.

The **glow** is first order: it falls exponentially, $g' = -g/\tau$. The exact solution over any interval $\Delta$ is $g\,e^{-\Delta/\tau}$, and exponentials compose, so $e^{-a}e^{-b}=e^{-(a+b)}$: chopping the same wall-clock time into 6 frames or 60 gives the same glow. That is what `stage.dt` is for. It is already the time since the last frame that moved forward, zero while paused and after a rewind, and capped at half a second, so the formula `1 - smoothing(dt, tau)` is all it needs.

The **spring** is second order, $\ddot x = -\omega^2 (x - x_\mathrm{target}) - 2\zeta\omega\dot x$, and an integrator for it is not exact. Semi-implicit Euler steps $v \leftarrow v + h\,a$, $x \leftarrow x + h\,v$. The spectral radius of that step map (computed from its trace and determinant) is what decides whether it is stable:

| Step $h$ | $\omega h$ | Spectral radius | |
|---|---|---|---|
| 0.02 s (`STEP`) | 0.15 | 0.91 | stable |
| 0.05 s | 0.38 | 0.77 | stable |
| 0.1 s (one frame at 10 fps) | 0.75 | 0.41 | stable, but a different path |
| 0.2 s | 1.51 | 2.23 | blows up |
| 0.5 s (the most `stage.dt` can be) | 3.77 | 16.5 | blows up |

with $\omega = 2\pi\cdot 1.2$ and $\zeta = 0.55$. So a spring cannot be stepped by `stage.dt`: it would explode on a stall, and even where it is stable its overshoot would depend on the frame rate. It is stepped by `simulation::FixedStep`, which turns wall-clock time into whole steps of a fixed size (`step`, at most `max_steps` per frame, at most `max_gap` seconds of gap counted, so a pause or a stall is dropped rather than caught up) and returns the fraction of the next step the frame lies into. Fed from `stage.time`, every frame rate follows the same trajectory. `Beads` holds both and is pure: it draws nothing, so a test can drive it at 10 and 60 frames per second and compare.

```rust
// src/lissajous.rs
/// A bead: where it is in plate units (0 to 1 each way), how fast it moves, the end of the
/// figure it is pulled towards, and how brightly it glows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bead {
    position: [f32; 2],
    velocity: [f32; 2],
    target: [f32; 2],
    glow: f32,
}

/// The beads' physics, free of drawing: a damped spring per process on a fixed clock, and a
/// glow that falls with time.
#[derive(Debug)]
struct Beads {
    clock: FixedStep,
    bodies: HashMap<Identity, Bead>,
    /// Steps taken so far, which tells two runs at different frame rates apart from nothing.
    steps: u64,
}

impl Default for Beads {
    fn default() -> Self {
        Self {
            clock: FixedStep::new(STEP, MAX_STEPS, MAX_GAP),
            bodies: HashMap::new(),
            steps: 0,
        }
    }
}

impl Beads {
    /// Pulls the bead of `id` towards `target` and lights it to at least `glow`. A bead seen for
    /// the first time starts on its target.
    fn pull(&mut self, id: Identity, target: [f32; 2], glow: f32) {
        let bead = self.bodies.entry(id).or_insert(Bead {
            position: target,
            velocity: [0.0; 2],
            target,
            glow,
        });
        bead.target = target;
        bead.glow = bead.glow.max(glow);
    }

    /// Lets every glow fall by `dt` seconds of wall-clock time. An exponential decay is the
    /// exact solution of its equation, so any chopping of the same time falls the same way.
    fn fade(&mut self, dt: f32) {
        let kept = 1.0 - smoothing(dt, PERSISTENCE);
        for bead in self.bodies.values_mut() {
            bead.glow *= kept;
        }
    }

    /// Steps every spring up to wall-clock `time`, in whole steps of `STEP`. The step map of
    /// semi-implicit Euler has spectral radius 0.91 at this step, 0.41 at a 10 fps frame's 0.1 s,
    /// 2.2 at 0.2 s and 16.5 at the 0.5 s that `stage.dt` can reach: only the fixed step is
    /// stable at every frame rate, and it gives every frame rate the same path.
    fn advance(&mut self, time: f32) {
        let (bodies, steps) = (&mut self.bodies, &mut self.steps);
        self.clock.advance(time, |step| {
            *steps += 1;
            for bead in bodies.values_mut() {
                for axis in 0..2 {
                    let pull = OMEGA * OMEGA * (bead.target[axis] - bead.position[axis])
                        - 2.0 * ZETA * OMEGA * bead.velocity[axis];
                    bead.velocity[axis] += pull * step;
                    bead.position[axis] += bead.velocity[axis] * step;
                }
            }
        });
    }
}
```

### Step 3: history in, figures out

A view needs the past of every process; it does not record it itself. `stage.history` is the shared `History`, a ring of the last 240 samples filled by the scene on every sample whichever view is shown, and a headless demo run replays it. A view asks for a window **in seconds** (`History::window(until, seconds)`), never a count, because `--sample-ms` ranges from 100 to 10000, and `until` is the shown snapshot's `elapsed`, so pause and rewind show the past they were taken in.

The figure is then pure maths on a series: `cpu_at` interpolates it (and holds it at its first reading before the process existed; "unknown is not zero", and a process has no CPU before it exists, so a young process starts on the diagonal rather than at the origin), `swing` ranks it, `curve` resamples it on a fixed grid so the cost does not depend on the sampling interval. `Figures::refresh` runs once per new sample (or delay), never per frame, and the ranking is over **every sampled process**, not the drawn ones, so hiding some never promotes another.

```rust
// src/lissajous.rs
/// What the history says about one process at the shown sample.
struct Figure {
    /// The CPU now and one delay ago, in percent: where the figure ends.
    head: [f32; 2],
    /// Highest minus lowest CPU in the trace, in percent.
    swing: f32,
    /// The whole figure, oldest first, as (CPU, CPU one delay before it); empty for a process
    /// that did not swing enough to earn one.
    curve: Vec<[f32; 2]>,
}

/// The CPU of a series of (elapsed, CPU) readings at `time`: linear between readings, and held at
/// the first before it, since a process has no CPU before it exists. The series is not empty.
fn cpu_at(series: &[(f64, f32)], time: f64) -> f32 {
    let after = series.partition_point(|&(at, _)| at <= time);
    match (after.checked_sub(1).map(|i| series[i]), series.get(after)) {
        (None, _) => series[0].1,
        (Some((_, last)), None) => last,
        (Some((from, before)), Some(&(to, next))) => {
            before + (next - before) * ((time - from) / (to - from)) as f32
        }
    }
}

/// Highest minus lowest CPU of a series.
fn swing(series: &[(f64, f32)]) -> f32 {
    let high = series.iter().map(|&(_, cpu)| cpu).fold(f32::MIN, f32::max);
    let low = series.iter().map(|&(_, cpu)| cpu).fold(f32::MAX, f32::min);
    high - low
}

/// The figure of one series ending at `now`: a point every `POINT_EVERY` seconds from the
/// process's first reading, so a young process has a short figure.
fn curve(series: &[(f64, f32)], now: f64, delay: f64) -> Vec<[f32; 2]> {
    let count = (TRACE / POINT_EVERY).round() as usize;
    (0..=count)
        .map(|k| now - (count - k) as f64 * POINT_EVERY)
        .filter(|&time| time >= series[0].0)
        .map(|time| [cpu_at(series, time), cpu_at(series, time - delay)])
        .collect()
}

/// The figures of every sampled process, rebuilt once per new sample (or delay).
#[derive(Default)]
struct Figures {
    built: Option<(u64, u64)>,
    by_process: HashMap<Identity, Figure>,
}

impl Figures {
    fn refresh(&mut self, history: &History, snapshot: &Snapshot, delay: f64) {
        let key = (snapshot.elapsed.to_bits(), delay.to_bits());
        if self.built == Some(key) {
            return;
        }
        self.built = Some(key);
        let now = snapshot.elapsed;
        let mut series: HashMap<Identity, Vec<(f64, f32)>> = HashMap::new();
        for sample in history.window(now, TRACE + delay) {
            for process in &snapshot.processes {
                if let Some(cpu) = sample.cpu(process.id) {
                    series
                        .entry(process.id)
                        .or_default()
                        .push((sample.elapsed, cpu));
                }
            }
        }
        let swung = |id: Identity| series.get(&id).map_or(0.0, |series| swing(series));
        // The ranking is over every sampled process, so hiding some never promotes another.
        let earners: HashSet<Identity> =
            top_n(&snapshot.processes, FIGURES, |p| f64::from(swung(p.id)))
                .into_iter()
                .filter(|p| swung(p.id) >= BUSY)
                .map(|p| p.id)
                .collect();
        self.by_process = series
            .iter()
            .map(|(&id, series)| {
                let figure = Figure {
                    head: [cpu_at(series, now), cpu_at(series, now - delay)],
                    swing: swing(series),
                    curve: if earners.contains(&id) {
                        curve(series, now, delay)
                    } else {
                        Vec::new()
                    },
                };
                (id, figure)
            })
            .collect();
    }
}
```

### Step 4: a layout that never moves

The harness fails a view whose processes move when CPU or memory changes, or when a tenth of the population exits and as many start. The tools are in `layout.rs`: `Seats` gives each process a seat number within its group (a freed seat goes to the next newcomer), `Discs` places named discs without overlap and keeps a disc where it is until it outgrows the room it reserved, and `spiral` places seat $k$ at distance $s\sqrt{k+o}$ and angle $k$ golden angles: consecutive seats never line up. The view asks `Discs` for a radius that grows smoothly with a group's seats (the spiral out to the last seat, plus a plate), and `Discs` reserves 15 % more than it is asked, so a group that gains a few processes keeps its place and only one that gains about a third again moves.

Two rules are easy to miss. First, seat and place **every sampled process** (`snapshot.processes`), not only the drawn slice, so that `f` (focus) or `--limit` hiding a process neither frees its seat nor moves the others; clearing it puts everything back. Second, the camera envelope (`stage.bounds`) must not depend on what is drawn either, or the camera would jump.

A test (below) also checks the one numerical claim the layout makes: that two plates on neighbouring seats never touch. The nearest seats of the spiral with offset 1 are 1.602 times its spacing apart (computed over the first 256 seats), which is 4.65 for a spacing of 2.9, and two axis-aligned squares of side 3.0 clear each other when their centres are 3.0 times the square root of two, 4.24, apart.

```rust
// src/lissajous.rs
/// A CPU in percent as a position along a plate's axis, 0 to 1.
fn along(cpu: f32) -> f32 {
    bounded(cpu, KNEE)
}

/// Plate coordinates of a point on the axes, inside the frame.
fn inset(unit: [f32; 2]) -> [f32; 2] {
    unit.map(|u| 0.5 + (u - 0.5) * PLOT)
}

/// Where plate coordinates `unit` (0 to 1 each way) lie in the world, `lift` above the ground.
fn world(centre: [f32; 2], unit: [f32; 2], lift: f32) -> Point {
    let (across, up) = (unit[0] - 0.5, unit[1] - 0.5);
    [
        centre[0] + SIDE * (across * ACROSS[0] + up * UP[0]),
        centre[1] + SIDE * (across * ACROSS[1] + up * UP[1]),
        lift,
    ]
}

/// The radius of the disc a group of `span` seats asks `Discs` for: the spiral out to its last
/// seat, and a plate. `Discs` reserves 15 % more than it is asked, so a group keeps its place
/// while it gains up to about a third as many processes again, and only then moves.
fn disc_radius(span: usize) -> f32 {
    SEAT_SPACING * (span as f32 + SEAT_OFFSET).sqrt() + SIDE
}
```

The state, and the methods that use it:

```rust
// src/lissajous.rs
#[derive(Default)]
pub struct Lissajous {
    seats: Seats,
    discs: Discs,
    /// Disc centres by group, and the sample they were laid out for.
    centres: HashMap<String, [f32; 2]>,
    laid_out: Option<u64>,
    figures: Figures,
    beads: Beads,
    /// How many times `m` was pressed: the delay is `DELAYS[presses % DELAYS.len()]`.
    presses: usize,
    /// For each process, the highest CPU it has used as of each sample that raised it.
    highs: HashMap<Identity, Vec<(f64, f32)>>,
    recorded: Option<f64>,
    /// Figures drawn in the last frame, for the legend.
    drawn: usize,
}

impl Lissajous {
    fn delay(&self) -> f64 {
        DELAYS[self.presses % DELAYS.len()]
    }

    /// Seats every sampled process in its group and places the groups, once per sample. Seats
    /// are kept for processes that are not drawn (focus, `--limit`), so clearing either puts
    /// every plate back where it was.
    fn lay_out(&mut self, snapshot: &Snapshot) {
        let key = snapshot.elapsed.to_bits();
        if self.laid_out == Some(key) {
            return;
        }
        self.laid_out = Some(key);
        self.seats
            .assign(snapshot.processes.iter().map(|p| (p.id, p.group.as_str())));
        let groups: BTreeSet<&str> = snapshot
            .processes
            .iter()
            .map(|p| p.group.as_str())
            .collect();
        let needs: Vec<(String, f32)> = groups
            .into_iter()
            .map(|group| (group.to_owned(), disc_radius(self.seats.span(group))))
            .collect();
        self.centres = self.discs.arrange(&needs);
    }

    /// The centre of `process`'s plate: its group's disc, then its seat on the spiral.
    fn plate(&self, process: &Process) -> Option<[f32; 2]> {
        let disc = self.centres.get(&process.group)?;
        let seat = self.seats.seat(process.id)?;
        let (distance, angle) =
            spiral(SEAT_SPACING, SEAT_OFFSET, seat as f32, spin(&process.group));
        Some([
            disc[0] + distance * angle.cos(),
            disc[1] + distance * angle.sin(),
        ])
    }

    /// The highest CPU `id` had used as of `until`, if it was recorded.
    fn high(&self, id: Identity, until: f64) -> Option<f32> {
        let steps = self.highs.get(&id)?;
        let after = steps.partition_point(|&(at, _)| at <= until);
        after.checked_sub(1).map(|i| steps[i].1)
    }

    /// What the inspector and the tour say about a process: its end point in the popup, the
    /// range it swung over in the notes under its sockets, and how to read the plate in the
    /// tour's callout.
    fn describe(&self, stage: &mut Stage, process: &Process, snapshot: &Snapshot, delay: f64) {
        let Some(figure) = self.figures.by_process.get(&process.id) else {
            return;
        };
        let [now, before] = figure.head;
        stage.popup.push((
            process.id,
            format!("figure ends at {now:.0}% now, {before:.0}% {delay} s ago"),
        ));
        stage.notes.insert(
            process.id,
            vec![format!(
                "swung {:.0}% over the last {TRACE:.0} s; peak since start {}",
                figure.swing,
                self.high(process.id, snapshot.elapsed)
                    .map_or("unknown".into(), |high| format!("{high:.0}%"))
            )],
        );
        if stage.toured == Some(process.id) {
            let reading = if figure.curve.is_empty() {
                "steady, on the diagonal"
            } else {
                "circling the diagonal"
            };
            stage.callout.push((process.id, reading.into()));
        }
    }
}
```

### Step 5: drawing, picking and the stage outputs

`draw` has an order. First it brings its own state up to date for the frame: the layout (once per sample), the figures (once per sample), the beads (`fade` by `stage.dt`, `pull` towards each drawn process's figure, `advance` by `stage.time`). Then it records the frame.

* **Picking.** A pickable item carries an index into `frame.identities`: the index of the process in the `processes` slice, which is the order of the frame. The plate's floor quad and the bead carry it, so clicking anywhere on a plate picks its process. Strokes made of triangles with pick `NONE` clear the pick where they draw, so the figure, the graticule and the peak tick are not pickable, as the conventions say.
* **`stage.positions`** gets the plate's centre for every drawn process. The tour, the labels, the exit flash and the harness's stability checks read it, so it is the seat, which is stable, and not the bead, which moves.
* **`stage.bounds`** is a fixed envelope: the corners of every group's reserved disc.
* **`stage.places`** holds the group labels. They are untrusted text, shaped by `label_name` here and passed through `terminal::clean_text` by the app before they reach the terminal.
* **`stage.tour`** asks the tour to visit the six most swinging processes, heaviest first.
* **`stage.popup`, `stage.notes`, `stage.callout`** are for the selected or toured process only: a line right after the process's own figures, a line after its sockets, and a line in the tour's callout. Building them for every process, every frame, would be waste.

```rust
// src/lissajous.rs
/// A plate's graticule: its frame, the lines at one core, and the diagonal where a steady
/// process sits.
fn graticule(frame: &mut Frame, camera: &Camera, centre: [f32; 2], color: Color) {
    let at = |u: [f32; 2]| world(centre, u, 0.06);
    let core = inset([along(100.0); 2])[0];
    let lines = [
        ([0.0, 0.0], [1.0, 0.0], 1.0),
        ([1.0, 0.0], [1.0, 1.0], 1.0),
        ([1.0, 1.0], [0.0, 1.0], 1.0),
        ([0.0, 1.0], [0.0, 0.0], 1.0),
        ([core, 0.0], [core, 1.0], 0.55),
        ([0.0, core], [1.0, core], 0.55),
        ([0.0, 0.0], [1.0, 1.0], 0.55),
    ];
    for (from, to, strength) in lines {
        frame.hairline(camera, at(from), at(to), tint(color, strength), 1.0);
    }
}
```

```rust
// src/lissajous.rs
impl ViewModule for Lissajous {
    fn draw(&mut self, stage: &mut Stage, processes: &[&Process], snapshot: &Snapshot) -> usize {
        let delay = self.delay();
        self.lay_out(snapshot);
        self.figures.refresh(stage.history, snapshot, delay);
        let drawn: HashSet<Identity> = processes.iter().map(|p| p.id).collect();
        self.beads.bodies.retain(|id, _| drawn.contains(id));
        self.beads.fade(stage.dt);
        for process in processes {
            let head = self
                .figures
                .by_process
                .get(&process.id)
                .map_or([along(process.cpu); 2], |figure| figure.head.map(along));
            self.beads
                .pull(process.id, head, bounded(process.cpu, KNEE));
        }
        self.beads.advance(stage.time);

        let camera = stage.camera;
        let frame = &mut *stage.frame;
        let mut figures = 0;
        let mut tour = Vec::new();
        let mut inspected: Vec<&Process> = Vec::new();
        let mut members: HashMap<&str, usize> = HashMap::new();
        for (index, process) in processes.iter().enumerate() {
            let Some(centre) = self.plate(process) else {
                continue;
            };
            let id = process.id;
            let pick = index as u32;
            let selected = stage.selected == Some(id);
            *members.entry(process.group.as_str()).or_default() += 1;
            stage.positions.insert(id, [centre[0], centre[1], 0.0]);

            let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]
                .map(|corner| world(centre, corner, 0.0));
            frame.quad(camera, corners, FLOOR, pick);
            graticule(
                frame,
                camera,
                centre,
                if selected { SELECTED } else { GRATICULE },
            );

            let base = kind_color(process);
            let figure = self.figures.by_process.get(&id);
            if let Some(figure) = figure.filter(|figure| !figure.curve.is_empty()) {
                figures += 1;
                let last = figure.curve.len().saturating_sub(2).max(1) as f32;
                let mut before: Option<Point> = None;
                for (k, cpus) in figure.curve.iter().enumerate() {
                    let at = world(centre, inset(cpus.map(along)), 0.10);
                    if let Some(from) = before {
                        // Older stretches are dimmer and thinner, like a phosphor trace.
                        let age = k as f32 / last;
                        frame.hairline(
                            camera,
                            from,
                            at,
                            tint(base, 0.18 + 0.82 * age),
                            1.0 + 1.6 * age,
                        );
                    }
                    before = Some(at);
                }
                tour.push(TourTarget {
                    id,
                    position: [centre[0], centre[1], 1.0],
                    weight: figure.swing,
                });
            }
            if let Some(high) = self.high(id, snapshot.elapsed) {
                let on = inset([along(high); 2])[0];
                let (a, b) = ([on - 0.05, on + 0.05], [on + 0.05, on - 0.05]);
                frame.hairline(
                    camera,
                    world(centre, [a[0], a[1]], 0.14),
                    world(centre, [b[0], b[1]], 0.14),
                    PEAK,
                    1.4,
                );
            }

            if let Some(bead) = self.beads.bodies.get(&id) {
                let at = world(centre, inset(bead.position), 0.30);
                let megabytes = (process.memory as f32 / 1_048_576.0).max(1.0);
                let radius = (0.07 + 0.03 * megabytes.log2()).min(0.26);
                frame.glow(
                    camera,
                    at,
                    2.0 + 14.0 * bead.glow,
                    base,
                    0.15 + 0.65 * bead.glow,
                );
                frame.sphere(camera, at, radius, base, pick, selected);
            }
            if selected || stage.toured == Some(id) {
                inspected.push(process);
            }
        }
        self.drawn = figures;
        for process in inspected {
            self.describe(stage, process, snapshot, delay);
        }

        tour.sort_by(|a, b| b.weight.total_cmp(&a.weight).then(a.id.cmp(&b.id)));
        tour.truncate(6);
        stage.tour.extend(tour);
        let mut named: Vec<(usize, &str)> = members.into_iter().map(|(g, n)| (n, g)).collect();
        named.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        for (_, group) in named.into_iter().take(NAMED) {
            if let Some(&[x, y]) = self.centres.get(group) {
                let reach = disc_radius(self.seats.span(group));
                stage.places.push((
                    camera.front([x, y, 0.0], reach),
                    label_name(group).into_owned(),
                ));
            }
        }
        for (group, &[x, y]) in &self.centres {
            let reach = disc_radius(self.seats.span(group));
            for corner in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
                let at = [x + corner[0] * reach, y + corner[1] * reach];
                stage
                    .bounds
                    .extend([[at[0], at[1], 0.0], [at[0], at[1], 1.0]]);
            }
        }
        processes.len()
    }

    fn record(&mut self, snapshot: &Snapshot, _time: f32) {
        if self.recorded.is_some_and(|last| {
            snapshot.elapsed.partial_cmp(&last) != Some(std::cmp::Ordering::Greater)
        }) {
            return;
        }
        self.recorded = Some(snapshot.elapsed);
        let alive: HashSet<Identity> = snapshot.processes.iter().map(|p| p.id).collect();
        self.highs.retain(|id, _| alive.contains(id));
        for process in &snapshot.processes {
            let steps = self.highs.entry(process.id).or_default();
            if steps.last().is_none_or(|&(_, high)| process.cpu > high) {
                steps.push((snapshot.elapsed, process.cpu));
            }
        }
    }

    fn legend(&self, _snapshot: &Snapshot) -> String {
        let delay = self.delay();
        format!(
            " {} figures | delay {delay} s | x = CPU now, y = CPU {delay} s ago | diagonal = steady | tick = peak since start | bead: size = memory, glow = CPU",
            self.drawn
        )
    }

    fn key(&mut self, key: ViewKey) -> bool {
        match key {
            ViewKey::Local('m') => self.presses += 1,
            ViewKey::Reset => self.presses = 0,
            ViewKey::Local(_) | ViewKey::Escape => {}
        }
        false
    }
}
```

### Step 6: `record`, `legend` and `key`

`record` is optional. Most views do not need one, because the shared history already holds their past. This one keeps something the history cannot: the highest CPU each process has used **since isotop began recording it**, which is not a window and not bounded by 240 samples. It must see every sample, whichever view is shown, which is exactly what `record` is for. Three rules from the conventions show in it: deduplicate on `snapshot.elapsed` (`record` may be offered the same sample twice, and `partial_cmp` handles NaN), prune identities that are no longer alive, and make the stored data answerable for the shown sample so rewind works. Storing the running maximum as a staircase of `(elapsed, high)` steps does all three: a binary search on `elapsed` answers for any time.

`legend` follows the order in the conventions: dynamic parts first (so they survive 80 columns), then degradation notes (none: this view reads no optional source, so there is nothing to put in `status::missing_note`), then the static mappings in `X = Y` form separated by ` | `. `key` handles the one view-local key, `m`, which steps the delay; Home and `r` reset it (the global reset happens as well). A view takes only the keys in `LOCAL_KEYS`; every other key, including all digits and capitals, is global.

### Step 7: the registry row

Wiring the module in takes one `mod` line in `main.rs` and one row at the end of `views::REGISTRY`. Set only what differs from the defaults:

```rust
// src/views.rs
    ViewInfo {
        // The scope's screen is black: the pressure haze would tint it.
        haze: false,
        // A top-down look, so a plate reads as a graph.
        pitch: Some(1.0),
        // Selected and toured processes show their links, as hairlines.
        link_strokes: true,
        keys: "m delay",
        // The peak-hold ticks come from the samples before the frame.
        warmup: &[Warmup {
            phase: Phase::Replay {
                seconds: 40,
                before_start: false,
            },
            when: When::DemoHeadless,
        }],
        // A figure is a trace and a delay long: 30 s and up to 8 s.
        contract: Contract {
            time: 40.0,
            ..Contract::DEFAULT
        },
        ..ViewInfo::new("lissajous", "LISSAJOUS", Backdrop::Void, || {
            Box::new(lissajous::Lissajous::default())
        })
    },
```

Every other field keeps its default. The row's fields, as `ViewInfo` documents them:

| Field | Default | In this row |
|---|---|---|
| `name`, `title` | | `lissajous`, `LISSAJOUS`: the lowercase CLI name and the status-line title |
| `backdrop` | | `Backdrop::Void`: black. The others are `Dusk`, `Space` and `Sea` |
| `available` | everywhere | default. A runtime probe lives in `platform/` |
| `build` | | A `views::Build`, here a closure that returns `Box::new(lissajous::Lissajous::default())`; the scene calls it once per row when it is created and owns the module from then on |
| `warmup` | none | a `Phase::Replay` of 40 s under `When::DemoHeadless`: a headless demo frame needs the samples before it for its peak ticks. A phase that replays demo samples must be `DemoHeadless` (or `DemoOutput`), because a live run must not take in demo samples; a new view's own phases use `Headless` so `--benchmark` measures a settled scene |
| `links` | `LinkRule::Quiet` | default: only the links of selected, hovered or toured processes, which join plate centres |
| `link_strokes` | off | on, as a new view must: links as hairlines |
| `flash` | on | default: an exit leaves the generic ring and glow where its plate was |
| `haze` | on | off: the screen is black, and the pressure haze would tint it |
| `pitch` | none | 1.0 rad, a steep look so a plate reads as a graph |
| `tour` | on | default |
| `header`, `face`, `turn` | none | default |
| `sources` | none | default: the view reads no optional source. A view that draws remote locations or the journal says so here and the background thread reads them only while it is shown |
| `demo` | `DemoOptions::default()` | default: `model::demo` exactly. A view that needs births and exits, exact CPU time or isotop's own process asks here (`churn`, `exact_cpu_time`, `observer`, `mortality`) |
| `keys` | none | `m delay`: the line that leads the `?` help while the view is shown |
| `contract` | `Contract::DEFAULT` | `time: 40.0`: the harness checks the view at 40 s of demo time, because its figure needs 30 s plus the longest delay, 8 s, of history. The other fields of the contract are `draws_processes`, `tolerance` (1 world unit), `anchors` and `known_unstable` |

Never add a `known_unstable` entry for a new view: those record debt of the views that predate the harness, each with its issue, and the harness reports one that starts to pass.

### Step 8: tests

The conventions ask for tests only of what the view owns beyond the harness. The harness (`every_view_satisfies_the_view_contract`, in `harness.rs`) already runs every registered view, headless, at its row's `contract.time`, and checks that the same inputs give the same pixels and picks, that the demo shows processes with positions and something to click, that every pick names a process or thing, that no process moves by more than the tolerance when CPU is scaled by 0.25 to 4 and memory by 0.5 to 2 or when a tenth of the processes exit and as many start, and that an empty machine draws. Registration alone is the coverage.

The view's own tests, as sentences about behaviour, use two things. `Scene::with_rows(ROWS)` over a table of one row of its own, as `views::testing` does, drives the module through the same trait calls the app makes (`record`, `render`, `legend`, `key`), without depending on the registry. And the pure parts (`Beads`, `curve`, `cpu_at`, the spiral) are driven directly at the frame rates and series the claim is about.

```rust
// src/lissajous.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::demo;
    use crate::model::testing::process;
    use crate::render::{Backdrop, NONE, RenderInput, Scene, hiding};
    use crate::views::{View, ViewInfo, view};

    /// The view on a row of its own with every optional field at its default, as `views::testing`
    /// builds its rows.
    static ROWS: &[ViewInfo] = &[ViewInfo::new(
        "lissajous",
        "LISSAJOUS",
        Backdrop::Void,
        || Box::new(Lissajous::default()),
    )];

    /// A scene over `ROWS` that has taken in the demo's first `seconds` seconds, a sample a
    /// second, and the sample of the last of them.
    fn lived(seconds: u32, count: usize) -> (Scene, Snapshot) {
        let mut scene = Scene::with_rows(ROWS);
        for second in 0..=seconds {
            scene.record(&demo(f64::from(second), count), second as f32);
        }
        (scene, demo(f64::from(seconds), count))
    }

    /// Records and rasterizes one frame of the only view of `ROWS`.
    fn draw(scene: &mut Scene, input: RenderInput) -> Frame {
        let mut frame = scene.render(&input);
        frame.rasterize();
        frame
    }

    fn small<'a>(snapshot: &'a Snapshot, camera: &'a Camera, time: f32) -> RenderInput<'a> {
        RenderInput::small(snapshot, View::at(0), camera, time)
    }

    #[test]
    fn a_delayed_sinusoid_draws_the_ellipse_of_a_lissajous_figure() {
        // x = a sin(t), y = a sin(t - d) satisfy x^2 - 2 x y cos(d) + y^2 = a^2 sin^2(d).
        let (mean, amplitude, period, delay) = (50.0_f32, 30.0_f32, 12.0_f64, 2.0_f64);
        let series: Vec<(f64, f32)> = (0..=600)
            .map(|k| {
                let time = f64::from(k) * 0.1;
                let phase = std::f64::consts::TAU * time / period;
                (time, mean + amplitude * phase.sin() as f32)
            })
            .collect();
        let d = (std::f64::consts::TAU * delay / period) as f32;
        let figure = curve(&series, 60.0, delay);
        assert!(figure.len() > 50, "{} points", figure.len());
        for [x, y] in figure {
            let (x, y) = (x - mean, y - mean);
            let left = x * x - 2.0 * x * y * d.cos() + y * y;
            let right = (amplitude * d.sin()).powi(2);
            assert!(
                (left - right).abs() < 0.01 * amplitude * amplitude,
                "{left} {right}"
            );
        }
    }

    #[test]
    fn a_steady_series_stays_on_the_diagonal_and_a_young_one_is_held_at_its_first_reading() {
        let steady: Vec<(f64, f32)> = (0..40).map(|k| (f64::from(k), 25.0)).collect();
        assert_eq!(swing(&steady), 0.0);
        assert!(curve(&steady, 39.0, 4.0).iter().all(|&[x, y]| x == y));
        let series = [(10.0, 20.0), (11.0, 40.0), (13.0, 0.0)];
        assert_eq!(cpu_at(&series, 5.0), 20.0, "held before the first reading");
        assert_eq!(cpu_at(&series, 10.5), 30.0, "linear between readings");
        assert_eq!(cpu_at(&series, 12.0), 20.0);
        assert_eq!(cpu_at(&series, 99.0), 0.0, "held after the last");
        assert_eq!(swing(&series), 40.0);
        let young = curve(&series, 13.0, 2.0);
        assert!(young.len() < 10, "no points before the process existed");
        assert_eq!(young[0], [20.0, 20.0]);
    }

    #[test]
    fn seats_on_the_spiral_keep_their_plates_apart() {
        let spot = |seat: usize| {
            let (distance, angle) = spiral(SEAT_SPACING, SEAT_OFFSET, seat as f32, 0.0);
            [distance * angle.cos(), distance * angle.sin()]
        };
        let nearest = (0..256)
            .flat_map(|a| (a + 1..256).map(move |b| (a, b)))
            .map(|(a, b)| {
                let (a, b) = (spot(a), spot(b));
                (a[0] - b[0]).hypot(a[1] - b[1])
            })
            .fold(f32::MAX, f32::min);
        // Two squares of side SIDE, turned any way, are apart when their centres are
        // SIDE * sqrt(2) apart.
        assert!(nearest > SIDE * std::f32::consts::SQRT_2, "{nearest}");
    }

    #[test]
    fn a_bead_follows_the_same_path_at_ten_and_sixty_frames_per_second() {
        let id = process(7, 1).id;
        let run = |fps: u32| {
            let mut beads = Beads::default();
            beads.pull(id, [0.1, 0.1], 0.0);
            beads.advance(10.0);
            beads.pull(id, [0.9, 0.4], 0.0);
            let mut states = HashMap::new();
            for k in 1..=3 * fps {
                beads.advance(10.0 + k as f32 / fps as f32);
                states.insert(beads.steps, beads.bodies[&id]);
            }
            states
        };
        let (slow, fast) = (run(10), run(60));
        let mut compared = 0;
        for (steps, bead) in &slow {
            if let Some(other) = fast.get(steps) {
                assert_eq!(bead, other, "after {steps} steps");
                compared += 1;
            }
        }
        assert!(compared >= 25, "compared {compared} states");
        let overshoot = fast.values().map(|b| b.position[0]).fold(0.0, f32::max);
        assert!(
            overshoot > 0.9 && overshoot < 1.0,
            "a damping ratio of 0.55 overshoots a little: {overshoot}"
        );
        let last = fast[fast.keys().max().expect("steps were taken")];
        assert!((last.position[0] - 0.9).abs() < 0.01 && (last.position[1] - 0.4).abs() < 0.01);
    }

    #[test]
    fn a_glow_rises_at_once_and_falls_the_same_way_at_any_frame_rate() {
        let id = process(7, 1).id;
        let fall = |fps: u32| {
            let mut beads = Beads::default();
            beads.pull(id, [0.5; 2], 1.0);
            for _ in 0..2 * fps {
                beads.fade(1.0 / fps as f32);
                beads.pull(id, [0.5; 2], 0.2);
            }
            beads.bodies[&id].glow
        };
        let (slow, fast) = (fall(10), fall(60));
        assert!((slow - fast).abs() < 1e-4, "{slow} {fast}");
        assert!(
            (slow - (-2.0_f32 / PERSISTENCE).exp()).abs() < 1e-3,
            "{slow}"
        );
        let mut beads = Beads::default();
        beads.pull(id, [0.5; 2], 0.1);
        beads.pull(id, [0.5; 2], 0.8);
        assert_eq!(beads.bodies[&id].glow, 0.8);
    }

    #[test]
    fn peak_hold_is_recorded_once_per_sample_pruned_and_readable_in_the_past() {
        let mut view = Lissajous::default();
        let sample = |elapsed: f64, pid: u32, cpu: f32| Snapshot {
            elapsed,
            processes: vec![Process {
                cpu,
                ..process(pid, 1)
            }],
            ..Snapshot::default()
        };
        let id = process(7, 1).id;
        view.record(&sample(1.0, 7, 10.0), 1.0);
        view.record(&sample(2.0, 7, 30.0), 2.0);
        view.record(&sample(2.0, 7, 99.0), 2.5);
        view.record(&sample(3.0, 7, 20.0), 3.0);
        assert_eq!(view.high(id, 0.5), None, "before the first sample");
        assert_eq!(view.high(id, 1.5), Some(10.0));
        assert_eq!(
            view.high(id, 2.5),
            Some(30.0),
            "the repeated sample was ignored"
        );
        assert_eq!(
            view.high(id, 9.0),
            Some(30.0),
            "a lower reading adds no step"
        );
        view.record(&sample(4.0, 8, 5.0), 4.0);
        assert_eq!(view.high(id, 9.0), None, "an exited process is forgotten");
    }

    #[test]
    fn every_process_gets_a_plate_and_only_the_busiest_sixty_four_a_figure() {
        let (mut scene, snapshot) = lived(40, 100);
        let camera = Camera::default();
        let frame = draw(&mut scene, small(&snapshot, &camera, 40.0));
        assert_eq!((scene.visible, scene.collapsed), (100, 0));
        assert_eq!(scene.positions.len(), 100);
        assert!(frame.picks.iter().any(|&pick| pick != NONE));
        let legend = scene.legend(View::at(0), &snapshot);
        assert!(legend.starts_with(" 64 figures | delay 2 s | "), "{legend}");
        assert!(!scene.places.is_empty() && scene.places.len() <= NAMED);
        assert!(scene.bounded(), "the camera fit has a fixed envelope");
        assert_eq!(scene.tour.len(), 6);
        assert!(
            scene
                .tour
                .windows(2)
                .all(|pair| pair[0].weight >= pair[1].weight)
        );
    }

    #[test]
    fn a_machine_at_rest_has_beads_on_the_diagonal_and_no_figures() {
        let mut scene = Scene::with_rows(ROWS);
        let steady = |second: u32| {
            let mut snapshot = demo(f64::from(second), 32);
            for process in &mut snapshot.processes {
                process.cpu = 5.0;
            }
            snapshot
        };
        for second in 0..=40 {
            scene.record(&steady(second), second as f32);
        }
        let snapshot = steady(40);
        let camera = Camera::default();
        draw(&mut scene, small(&snapshot, &camera, 40.0));
        let legend = scene.legend(View::at(0), &snapshot);
        assert!(legend.starts_with(" 0 figures"), "{legend}");
        assert!(scene.tour.is_empty());
    }

    #[test]
    fn the_inspector_and_the_tour_get_their_lines_and_m_changes_the_delay() {
        let (mut scene, snapshot) = lived(40, 64);
        let camera = Camera::default();
        let id = snapshot.processes[3].id;
        draw(&mut scene, small(&snapshot, &camera, 40.0));
        assert!(scene.popup.is_empty() && scene.callout.is_empty() && scene.notes.is_empty());
        let chosen = RenderInput {
            selected: Some(id),
            toured: Some(id),
            ..small(&snapshot, &camera, 40.1)
        };
        draw(&mut scene, chosen);
        assert!(
            scene
                .popup
                .iter()
                .any(|(who, line)| *who == id && line.starts_with("figure ends at"))
        );
        assert!(scene.notes[&id][0].starts_with("swung "));
        assert_eq!(scene.callout.len(), 1);
        let legend = |scene: &Scene| scene.legend(View::at(0), &snapshot);
        for (key, delay) in [('m', "4"), ('x', "4"), ('m', "8"), ('m', "1"), ('m', "2")] {
            scene.key(View::at(0), ViewKey::Local(key));
            assert!(
                legend(&scene).contains(&format!("delay {delay} s")),
                "after {key}"
            );
        }
        scene.key(View::at(0), ViewKey::Local('m'));
        scene.key(View::at(0), ViewKey::Reset);
        assert!(legend(&scene).contains("delay 2 s"));
        assert!(
            !scene.key(View::at(0), ViewKey::Escape),
            "Esc keeps its global meaning"
        );
    }

    #[test]
    fn plates_and_the_camera_envelope_ignore_what_focus_and_the_limit_hide() {
        let sample = demo(30.0, 160);
        hiding::focus_and_clear(view("lissajous"), &sample);
        hiding::limit_and_lift(view("lissajous"), &sample);
        hiding::bounds_ignore_hiding(view("lissajous"), &sample);
    }
}
```

The two tests that follow the conventions' model most closely are `a_bead_follows_the_same_path_at_ten_and_sixty_frames_per_second`, which compares states at equal step counts exactly (the model for any simulation view, after `flock_follows_the_same_path_at_ten_and_sixty_frames_per_second` in the coop), and `plates_and_the_camera_envelope_ignore_what_focus_and_the_limit_hide`, which calls the shared `render::hiding` helpers on the registered view.

### Step 9: everything else you must touch

The Rust is half of a view. This list is what the compiler, the tests and the checks actually refused until it was done, on the checkout described under [The gates](#the-gates). `git status` there showed exactly these files, except where a row says it was not done.

| File | Change | Refused until it was done |
|---|---|---|
| `src/lissajous.rs` | New: the module and its tests | |
| `src/main.rs` | `mod lissajous;`, alphabetically among the other `mod` lines | Nothing compiles the file without it |
| `src/views.rs` | Add `lissajous` to the `use crate::{...}` list; append the row at the end of `REGISTRY` | The view does not exist for Tab, `--view`, `--list-views` or the picker without it |
| `AGENTS.md` | A row for `lissajous.rs` in the file table | `agents_file_table_lists_every_source_file` |
| `tests/golden.txt` | One line, added by `ISOTOP_BLESS=1 cargo test every_view_matches`; the eighteen existing lines did not change, which is the proof that nothing shared was touched | `every_view_matches_its_golden_hash`: `lissajous: no golden hash` |
| `scripts/smoke_terminal.py` | `"lissajous"` appended to `PICKER_VISITS`, so the smoke test visits the view through the `v` picker | The conventions; the script skips names `--list-views` does not print |
| `README.md`, `docs/METRICS.md` | The metrics block and `docs/METRICS.md` regenerated by `python3 scripts/code_report.py` (needs `tokei`). A real view also needs a README `### Name` section under `## Worlds` (image, one sentence, the mapping table, **Decorative:**, the model, caps, what cannot be measured, keys), its name in the `--view` list, the Tab line in Controls and the headless paragraph, and its key in the key table, and the view counts the README states in prose. **Not done for the tutorial's view** except the regenerated blocks | `python3 scripts/code_report.py --check`, the last gate |
| `docs/media/lissajous.webp` | The README image: `--demo --view lissajous --time 40 --width 1600`, converted with ImageMagick. **Not done** | The conventions |

Two edits the first version of this page listed are no longer needed, and the reason is worth knowing. `top_n` no longer carries an `#[expect(dead_code)]`, so there is nothing to delete when a view first calls it (`status::sources_note` and `History::series` still carry one, and the first view to call either has that edit to make). And the two picker tests in `main.rs` no longer hard-code the registry's length and last row (they read `REGISTRY.len()` and `REGISTRY.last()`), so a new row needs no edit there. Likewise `every_row_builds_its_own_module_and_only_the_matrix_replaces_the_inspector` in `views.rs` checks every row generically; it needs no new entry for a view that draws processes.

What did **not** need touching, and why: `.github/workflows/ci.yml` renders every view `--list-views` prints, so the new view is rendered by CI without an edit; Tab, Shift+Tab, `--view`, `--views` and the picker follow from the table; the digits stay bound to the first ten rows. A view that reads an optional source also needs a `Source` variant in `platform/mod.rs` and a read in the sampler; this one reads none.

Not done, and a pull request would have to: the CPU/GPU parity run (`scripts/compare_png.py` on a `--renderer cpu` and a `--renderer gpu` render, within the default tolerance for a new view; there is no GPU here), `cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings`, a look in a real Ghostty, and the PR description with `## Plan` and `## Decisions`.

## The gates

The page was first written before PR 143. It was re-checked on a throwaway worktree of `main` at commit a34733c (Linux x86_64, run on 2026-10-13): the listings on this page were extracted and applied exactly as the steps say, and the gates run in the order `AGENTS.md` gives. What happened, in order:

| Step | Result |
|---|---|
| Apply the page as first written | `cargo check --all-targets` failed twice: `no Hooks in views` in the registry row and again in the tests' `ROWS` (the `Hooks::Module(...)` wrapper is gone; the listings above now pass the closure directly) |
| `cargo fmt --check` | Two diffs, both in `src/views.rs`: the long `use crate::{...}` line, and `ViewInfo::new(...)` laid out over several lines. `cargo fmt` fixed them; the listings above show the formatted form. `src/lissajous.rs` needed none |
| `cargo clippy --all-targets -- -D warnings` | Clean, first time |
| `cargo test lissajous` | `10 passed` |
| `cargo test` | First run: 2 failed, 847 passed, 9 ignored: `agents_file_table_lists_every_source_file` (no row) and `every_view_matches_its_golden_hash` (`lissajous: no golden hash`). `every_view_satisfies_the_view_contract` passed with the new row, first time. After the AGENTS row and `ISOTOP_BLESS=1 cargo test every_view_matches`: `849 passed; 0 failed; 9 ignored` |
| `cargo build --release` | Built |
| `python3 scripts/smoke_terminal.py <release binary>` | Passed in demo mode (44 valid RGB frames over shared memory) and live mode (43 frames, inline zlib), with `lissajous` among the picker visits |
| `python3 scripts/code_report.py --check` | Failed as expected before regenerating (the README block and `docs/METRICS.md` were stale); passed after `python3 scripts/code_report.py`: 64,874 lines of Rust, 878 tests, 19 views |
| macOS clippy, CPU/GPU parity, a real Ghostty | **Not run** |

Two further facts from the same run. Two renders of the same arguments are the same file (`cmp`), and the golden line the view added is `lissajous 12acd7b0d9b19af1 9b72d24fbf1bcb21 e9d31a91b1ccc61f`, the same as on the first version of this page. The CPU rasterizer measured 8.97 ms a frame at 192 processes (111 fps, `--width 1920`, `--demo`, `--benchmark 60`) and 19.03 ms a frame at `--processes 1000` (52 fps), against the conventions' budget of at least 10 and 5 fps; the machine differs from the first measurement (14.76 and 27.24 ms), so the two sets are not comparable.

The first version of this page also recorded what the harness caught in the author's first draft, and that story is still the best illustration of the point:

```
views break the view contract:
  lissajous: churn (76 of 173 surviving processes moved more than 1: pid 1112 by 146.29, ...)
```

The first draft of `disc_radius` rounded a group's seats up to the next power of two, so that a group would keep its disc until it doubled. In the demo every group has exactly 16 members, so a group that gained a single process doubled its reservation (radius about 1.3 times larger, more than the 15 % `Discs` allows), the disc was placed again, and all of its plates moved. The harness's churn run starts a tenth of the processes as siblings of survivors and caught it at once. The fix is the smoother radius in the listing above, which lets `Discs`' own headroom do the work. (The listing passes today; this failure belongs to the draft it replaced.)

## Look at it

```sh
./target/release/isotop --demo --view lissajous --width 1600 --time 40 --renderer cpu --output lissajous.png
```

`--time 40` is the contract's reference time: the figure needs 30 s of trace and the longest delay is 8 s. At the default `--time 12` the same command draws shorter figures, because the process has no more than twelve seconds of history to draw.

What the render shows (I looked at it; it is not reproduced on this page, because a toy view that is not on `main` has no media URL): twelve discs of sixteen plates, one per group of the demo's twelve families, on a black screen. Each plate is a dark teal square with a frame, a cross at one core and a diagonal. Busy processes have a coloured loop in their kind's colour (teal system services, amber session, violet containers, slate kernel threads), brighter towards now, with the bead on it and a white tick on the diagonal where the process peaked. Idle processes are a bead near the corner and a tick. Pink and red beads are zombies and blocked processes, by the shared palette.

Honest criticism of the picture, which a reviewer would make: at 1600 pixels each plate is about 22 pixels a side, so the loops are small; a terminal at full width does better, and Ctrl-scroll zooms. The demo's loops are all one shape (every process has the same period and a different phase and scale), so the field is repetitive where a real machine would not be. The beads of idle processes sit on the plate's corner and are the largest thing on it. None of these is a reason for the contract to fail, and each is a variation to try.

In an interactive session, `m` steps the delay and the loops reshape at once (the ellipse formula above says how); selecting a plate adds its figures to the inspector popup; `g` tours the six most swinging processes.

## What a real plugin interface might look like (speculation)

Everything in this section is speculation. isotop has no plugin interface, no dynamic loading, no scripting layer and no foreign-function boundary for views, and nothing here is on the roadmap. It is a design note: if someone wanted to write a view without rebuilding isotop, what would the interface be, and what would it cost? The reasoning is anchored in what the code already is.

### What the boundary already looks like

The view contract is narrow on purpose. A view reads three things, `Stage` (the frame, the camera, the selection, time, `dt`, the shared history), the process slice and `&Snapshot`, and talks back only through `Stage` outputs (`positions`, `places`, `bounds`, `notes`, `tour`, `popup`, `callout`, plus the display list it records into the frame). Its state is its own. It never reads the operating system, never opens a file or a socket, and never touches the terminal. Every drawing call lands in a display list of screen-space `Item`s (triangles, lines, sphere impostors, glows, beams, glyphs, pick discs) that two rasterizers agree on. That is nearly a serialisable function:

```
(snapshot, history window, selection, camera, size, time, dt, key events)
        -->  (display list, positions, places, bounds, notes, tour, popup, callout, legend)
```

Three things stop it being a function today: the view keeps state between frames (its struct), it draws by calling `Frame` methods that need a `&mut Frame` and a `&Camera` rather than returning data, and `Stage` holds references into the scene's own maps. All three are the kind of thing an interface layer can paper over, and none of them is an obstacle in principle.

### Option 1: a WebAssembly module per view

The host (isotop) would load a `.wasm` file exporting `init`, `record`, `draw`, `legend` and `key`, and import a small set of host functions (`sphere`, `hairline`, `glow`, `quad`, `pick_for`, `push_place`, ...). `draw` would receive the process slice and snapshot as a bounded linear-memory buffer written by the host in a fixed layout, and call the host's drawing imports.

For: the sandbox is real (no ambient file, network or process access), which fits the no-network and untrusted-text invariants; WebAssembly's floating-point semantics are close to deterministic, so the golden-hash discipline could survive (it would need checking: NaN payloads and `sin`/`exp` come from the module's own libm, which is actually an improvement over the host's); modules are portable across Linux and macOS, so one `.wasm` runs on both. Against: it adds a dependency, an interpreter or compiler, which the project's rule ("add a dependency only when it replaces substantial, error-prone code") makes a hard sell, and a JIT would sit awkwardly beside the `unsafe` policy; the per-frame cost of copying a snapshot of a thousand processes into guest memory is small but real; the host's drawing imports become an ABI that must be versioned for ever; and every host-side data structure a view reads today (the `History` ring, `Seats`, `Discs`, `FixedStep`, `top_n`) would have to be re-exposed or re-implemented in the guest language.

### Option 2: a scripting layer

Embed a small interpreter and let a script define the same hooks. For: no toolchain for the author, and live reloading would be pleasant for experiments. Against: the same dependency objection; scripts run per frame over up to a thousand processes at 10 frames per second on the CPU budget the views conventions set (10 fps at 1080p with 192 processes, 5 fps at 1000), and an interpreter inside the frame loop spends that budget quickly; the golden-hash and parity checks are defined on the Rust code path; and a script is the easiest way to read `/proc` or the network, which the "sampling lives in `platform/`" and "no network" invariants forbid, so the sandbox would need to be designed rather than assumed. The conventions, `harness.rs` in particular, are Rust tests that run every registered view, so a script view would have to be registered into the same table to be checked at all.

### Option 3: a stable draw-list protocol over a pipe

Run the view as a separate process. isotop writes the inputs to the child's stdin and reads a draw list from its stdout; the child can be written in any language. This is the shape of [issue 97](https://github.com/tfpickard/isotop/issues/97) (planned, not implemented): `isotop --emit` writes snapshots as a compact binary stream on stdout and `isotop --stdin` renders a stream, so that `ssh host isotop --emit | isotop --stdin` renders on the local GPU. That issue depends on a versioned snapshot codec ([issue 91](https://github.com/tfpickard/isotop/issues/91): std-only, little-endian, LEB128 varints, a magic header and a format version, keyframes and deltas, bounded lengths, a fixture file per released version) and on a `SnapshotSource` trait that puts the live collector, the demo and a replay behind one interface ([issue 90](https://github.com/tfpickard/isotop/issues/90)). A view process would be the other half of the same idea: the snapshot stream in, a draw-list stream out.

For: isolation is by process, so a crashing or slow view cannot take the host down and the OS can sandbox it; any language works; no new dependency in isotop, which fits the "codecs stay in-tree" decision of issue 91; and the host keeps ownership of everything that has to agree between the CPU and GPU rasterizers, since the child only emits `Item`s. Against: a round trip per frame (a snapshot of a thousand processes in, a few thousand items out) must fit a 100 ms frame at 10 fps and ideally much less; picking needs the child to tag items with indices into the host's identity table, and the host has to trust those indices (`ValidPicks` in the harness would have to run on the child's output); text from the child is untrusted exactly as process names are, and would go through the same `terminal::clean_text`; the child has no `History` unless the host sends a window of it every frame or the child keeps its own, which is what `record` is for; and the stable-layout invariant becomes the author's problem with no compiler to help, so the harness would have to run against the child as a black box (it already treats a view that way: it feeds samples and looks at pixels and positions).

### Trade-offs, in one table

| | Compiled in (today) | WASM | Script | Draw-list over a pipe |
|---|---|---|---|---|
| New dependency in isotop | none | an engine | an interpreter | none |
| Sandbox | none needed: reviewed code | by construction | must be designed | by the OS |
| Per-frame overhead | none | copy in, call out | interpreter cost | pipe round trip |
| Authors need | Rust and a rebuild | a wasm toolchain | the script language | any language |
| Golden hashes and parity | exact | probably exact | not defined | host-side, if the child is deterministic |
| Shared helpers (`Seats`, `History`, `FixedStep`) | direct | re-exported or rewritten | re-exported | rewritten by the author |
| Harness coverage | by registration | needs a host shim | needs a host shim | black box |

### What I would try first

Not any of the three, yet. The cheapest honest step towards any of them is to make the contract literally a function: give `draw` an input struct and an output struct (the display list as a value, not as calls on a `&mut Frame`), and run the existing views through it. That costs nothing in features, can be tested by the existing golden hashes (the frames must not change), and it is the common prerequisite of every option above. After that, issue 97's codec gives the inputs a wire format, and a draw-list codec is the symmetrical half. Whether to expose that to anyone but the project's own agents is a separate decision about whether the project wants third-party code in the frame loop at all, which the owner's stated aim (maximalism for its own sake) does not obviously ask for. The reasonable status quo is the one this page documents: a view is a Rust module and one row of `REGISTRY`, and the way to add one is a pull request.

The Lissajous view has no use, and ten tests.
