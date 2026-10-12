# Glossary

Terms as isotop and this wiki use them. Entries marked *(isotop)* name something in the code; the page they
link to says exactly how. Entries marked *(general)* are standard physics, mathematics or computing, defined
here only as far as the wiki needs.

## isotop: data

**Background sampler** *(isotop)*: the thread that reads the slow sources (sockets, open files, file locks,
cgroup accounting, NVIDIA memory) every `BACKGROUND_PERIOD` (2 s) so the frame loop never waits on them.
[Theory: Background Sampler and Demand](Theory-Background-Sampler-and-Demand).

**Collector** *(isotop)*: the object that samples the operating system once per `--sample-ms`, smooths CPU use
and returns a `Snapshot`. [Theory: Model and Rates](Theory-Model-and-Rates).

**Demand** *(isotop)*: the set of optional `Source`s the currently shown view declares it needs. The background
thread reads an optional source only while it is demanded. [Theory: Background Sampler and Demand](Theory-Background-Sampler-and-Demand).

**Identity** *(isotop)*: what names a process across samples: not the pid alone, since pids are reused. Layouts,
seeds, history and the background merge are all keyed by it, so a reused pid inherits nothing.
[Theory: Model and Rates](Theory-Model-and-Rates), [Theory: Stable Layout](Theory-Stable-Layout).

**Life event** *(isotop)*: a fork, exec or exit, from the proc connector and taskstats on Linux or kqueue on
macOS, merged with sample diffing into `Snapshot.life`. One exit per `Identity`. [Theory: Life Events](Theory-Life-Events).

**History** *(isotop)*: the shared ring of per-process CPU and memory samples, recorded by the scene on every
sample whichever view is shown. [Theory: History](Theory-History).

**Missing** *(isotop)*: `Snapshot::missing`, the optional sources a view demanded and the platform could not
read. A view says so in its status strip instead of drawing a made-up number.

**Snapshot** *(isotop)*: one sample of the machine: processes, system figures, which sources were read and
which were missing, life events. [Theory: Model and Rates](Theory-Model-and-Rates).

**Unknown is not zero** *(isotop)*: a reading that was not taken (the first sample of a rate, a counter that
went backwards, an unreadable file) is `None`, not 0. A view converts it only at its own boundary.
[Theory: Model and Rates](Theory-Model-and-Rates), [Philosophy](Philosophy).

## isotop: scene and pixels

**Display list** *(isotop)*: the screen-space `Item`s a view records for a frame: triangles, lines, sphere
impostors, glows, beams, stars. [Theory: Scene and Display List](Theory-Scene-and-Display-List).

**Frame** *(isotop)*: the target of a render: colour, depth and a pick buffer. The CPU and the GPU rasterizers
both fill one. [Theory: Rasterization](Theory-Rasterization).

**Golden hash** *(isotop)*: a recorded hash of each view's CPU render in `tests/golden.txt`, checked by
`cargo test` on Linux x86_64. [Theory: Determinism](Theory-Determinism).

**Hairline** *(isotop)*: `Frame::hairline`, a thin stroke drawn as short pieces of triangles (at most 8 px each),
which keeps the cost linear in length and lets the CPU and GPU rasterizers fill it alike. Views whose registry
row sets `link_strokes` draw their links this way. [Theory: Rasterization](Theory-Rasterization),
[Theory: GPU and Parity](Theory-GPU-and-Parity).

**Kitty graphics protocol** *(general)*: the terminal escape-sequence protocol isotop uses to show frames, over
POSIX shared memory (`t=s`) or inline as zlib and base64. [Theory: Terminal Graphics](Theory-Terminal-Graphics).

**Parity (CPU/GPU)** *(isotop)*: the invariant that the CPU rasterizer and the WGSL shader draw the same frame,
checked against a tolerance. [Theory: GPU and Parity](Theory-GPU-and-Parity).

**Pick buffer** *(isotop)*: a per-pixel buffer naming what is under the pixel, either an index into
`frame.identities` or a `THING`-bit index into `frame.things`. [Theory: Picking](Theory-Picking).

**Sphere impostor** *(isotop)*: a sphere drawn as a screen-space quad whose pixels are shaded analytically from
the ray-sphere intersection, not as a mesh. [Theory: Scene and Display List](Theory-Scene-and-Display-List).

**Stage** *(isotop)*: what a view reads and writes while drawing: the frame, camera, selection, `dt`, history,
and the outputs (positions, bounds, tour targets, popup lines). [Theory: Overview](Theory-Overview).

**Thing** *(isotop)*: a pickable that is not a process, with its own inspector card. [Theory: Picking](Theory-Picking).

## isotop: views and time

**`dt` policy** *(isotop)*: `stage.dt` is 0 on the first frame, while paused and when time goes back, otherwise
the time since the last forward frame, at most `MAX_DT` (0.5 s). [Theory: Overview](Theory-Overview).

**FixedStep** *(isotop)*: a fixed-step clock that runs whole simulation steps and returns the interpolation
fraction, so a simulation advances in wall-clock time independent of frame rate.
[Theory: Simulation Toolkit](Theory-Simulation-Toolkit).

**Realness contract** *(isotop)*: every visual element maps to a measured quantity, or the documentation marks
it decorative. [Philosophy](Philosophy).

**Registry (`REGISTRY`)** *(isotop)*: the table with one row per view, in Tab order; the digits pick the first
ten rows. [Guide: Writing a View](Guide-Writing-a-View).

**Stable layout** *(isotop)*: a process keeps its place when CPU, memory or the population changes; enforced by
the view harness. [Theory: Stable Layout](Theory-Stable-Layout).

**View module** *(isotop)*: a view implemented as a state struct plus `views::ViewModule`, with a registry row.
Older views are `Hooks::BuiltIn` and being migrated. There is no dynamic plugin loading.
[Guide: Writing a View](Guide-Writing-a-View).

## Operating system

**cgroup v2** *(general)*: the Linux hierarchy that groups processes and accounts for their CPU, memory and I/O.
[View: Cells](View-Cells), [Theory: Sampling on Linux](Theory-Sampling-Linux).

**Jiffy / `USER_HZ`** *(general)*: the unit of CPU time in `/proc/<pid>/stat`: clock ticks, converted to seconds
by the tick rate. [Theory: Sampling on Linux](Theory-Sampling-Linux).

**Proc connector** *(general)*: a netlink interface that delivers fork, exec and exit notifications.
[Theory: Life Events](Theory-Life-Events).

**PSI (pressure stall information)** *(general)*: kernel counters for time tasks spent stalled on CPU, memory or
I/O. [View: Cells](View-Cells).

**Taskstats** *(general)*: a netlink interface that delivers per-task accounting at exit.
[Theory: Life Events](Theory-Life-Events).

## Mathematics and physics

**Courant–Friedrichs–Lewy (CFL) condition** *(general)*: the bound on the time step for a stable explicit wave
scheme. [Math: Wave Equation](Math-Wave-Equation).

**Deferent and epicycle** *(general)*: in Ptolemaic astronomy, the large circle a planet's centre rides and the
small circle the planet rides on it. A chain of epicycles is a Fourier series.
[Math: Fourier Epicycles](Math-Fourier-Epicycles), [View: Ptolemy](View-Ptolemy).

**Discrete Fourier transform (DFT)** *(general)*: the finite transform that gives the epicycle coefficients of a
sampled closed curve. [Math: Fourier Epicycles](Math-Fourier-Epicycles).

**Gibbs phenomenon** *(general)*: the overshoot of a truncated Fourier series near a discontinuity.
[Math: Fourier Epicycles](Math-Fourier-Epicycles).

**Lorentz boost** *(general)*: the hyperbolic rotation of the time–space plane between inertial frames in
special relativity. In the Tesseract view it is a display transformation into a process's rest frame, which a
process using more than one core does not have. [Math: Lorentz and Schlegel](Math-Lorentz-and-Schlegel),
[View: Tesseract](View-Tesseract).

**Order parameter** *(general)*: a scalar that measures collective alignment in a flocking model (in the Vicsek
model, the length of the mean heading vector). [Math: Flocking](Math-Flocking).

**Parseval's theorem** *(general)*: the energy of a signal equals the energy of its transform; it ranks which
epicycles matter most. [Math: Fourier Epicycles](Math-Fourier-Epicycles).

**Pogson magnitude** *(general)*: the logarithmic brightness scale, `m = -2.5 log10(F)` up to a zero point.
[Math: Stellar Photometry](Math-Stellar-Photometry).

**Retrograde motion** *(general)*: the apparent backward motion of a planet against the stars, explained
geometrically by an epicycle. [View: Ptolemy](View-Ptolemy).

**Schlegel diagram** *(general)*: a projection of a polytope into one lower dimension through a point just
outside one facet. [Math: Lorentz and Schlegel](Math-Lorentz-and-Schlegel).

**Schwarzschild radius** *(general)*: `r_s = 2GM/c^2`, the horizon radius of a non-rotating mass.
[Math: Schwarzschild](Math-Schwarzschild), [Planned: Horizon](Planned-Horizon).

**Stigmergy** *(general)*: coordination through traces left in a shared environment, as ants do with pheromone.
[Planned: Anthill](Planned-Anthill).

**Turing instability** *(general)*: a diffusion-driven instability in which a stable uniform state of a
reaction–diffusion system becomes unstable once the species diffuse at different rates.
[Math: Reaction-Diffusion](Math-Reaction-Diffusion).

**Vicsek model** *(general)*: self-propelled particles that align with neighbours' mean heading plus noise;
shows an order–disorder transition. [Math: Flocking](Math-Flocking), [View: Coop](View-Coop).
