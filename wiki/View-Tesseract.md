# View Tesseract

Every process is a worldline through four dimensions over the last minute, drawn inside a wireframe tesseract in Schlegel projection: the outer cube is *now*, the inner cube is a minute ago. The fourth axis is time, and the view takes that literally: because the CPU-seconds
axis and the time axis share a scale, the slope of a worldline is the process's CPU use in cores, a process at exactly one core runs along the light cone, and a
multithreaded process that uses more than one core goes "superluminal" and trails a blue Cherenkov cone. A Lorentz boost into a process's rest frame is physics, not camera: it
re-describes the same data, and the camera's four-dimensional rotation is separate.

![Tesseract: worldlines of the last minute in a Schlegel-projected tesseract, superluminal processes leaving through the +x cell in Cherenkov blue](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/tesseract.webp)

![The README image of the same view](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/tesseract.webp)

## The system being modelled

**Spacetime as a plot.** In special relativity (Einstein 1905) an event has four coordinates; Minkowski (1908) showed that the invariant of the theory is the *interval* $\Delta s^2 = c^2\Delta t^2 - \Delta x^2$, and that a particle's history is a *worldline* through
that space. A particle at rest has a vertical worldline; its slope against the time axis is its velocity; light has slope $\pm c$, the *light cone*; matter slower than light stays inside it. A change of inertial frame is a *Lorentz boost*, a hyperbolic rotation of the $(ct, x)$ plane that preserves
$\Delta s^2$ and tilts both the time and space axes towards the light cone by the same angle. Hinton (1888) gave the four-dimensional cube its name, *tesseract*; a Schlegel diagram (1886) draws a polytope in one fewer dimension by perspective, so that a tesseract becomes a cube
inside a cube. Mathematical details, with derivations and figures, are on [Math Lorentz and Schlegel](Math-Lorentz-and-Schlegel).

**Why it fits a process.** A process's accounting gives a cumulative CPU time. Plotted against wall-clock time its slope is the number of cores the process used. That slope has a real, hard bound for a single thread: it can run on one CPU at a time, so one thread's CPU time cannot grow faster than the wall clock. The bound is a light cone: one core is $c$. Only multithreaded
processes can exceed it. That is an accurate statement about the machine, not a pun; what is only a metaphor is the *physics* of relativity: a process's CPU time is not frame-dependent, so boosting is a display transformation, and the view says so.

## History

The longer account lives in the repository: [docs/theory/tesseract.md](https://github.com/tfpickard/isotop/blob/main/docs/theory/tesseract.md).

**A fourth dimension.** Mathematicians had long used four coordinates as algebra; the geometry arrived in the 1850s with Ludwig Schlafli, who found the regular polytopes of every dimension (six in four dimensions, among them the 4-cube). Charles Howard Hinton popularised the idea and, in *A New Era of Thought* (1888), coined the word tesseract, from the Greek for "four rays", the four edges meeting at each vertex. He taught readers to picture four dimensions with sets of coloured cubes. He had confessed to bigamy to the police in 1886, was convicted, and left England for Japan in 1887, which is where he spent the years that followed. Victor Schlegel's diagrams of polytopes, projected so that one cell contains the rest, date from the 1880s.

**Time as the fourth.** Hendrik Lorentz (1904) found the transformations under which Maxwell's equations keep their form, and Einstein (1905) derived them from the principle of relativity and the constancy of the speed of light. In 1908 Hermann Minkowski, once Einstein's teacher, recast the theory as geometry: "Henceforth space by itself, and time by itself, are doomed to fade away into mere shadows". Alfred Robb (1911) introduced rapidity as the natural parameter of a boost. Salvador Dali painted an unfolded tesseract as the cross in *Corpus Hypercubus* (1954).

**Cherenkov.** Pavel Cherenkov observed a faint blue glow from radioactive sources in liquids in 1934; Ilya Frank and Igor Tamm explained it in 1937 as radiation from a particle faster than light in that medium, and the three shared the 1958 Nobel Prize in Physics.

## Theory

The coordinates of a worldline point are $(x, y, z, w)$, all in tesseract units where the cube is $[-1, 1]^4$:

$$
w = 1 + \frac{t - t_\text{now}}{30\,\mathrm{s}},\qquad
x = -1 + \frac{\text{CPU-seconds since the window opened}}{30},\qquad
y = -1 + 2\sqrt{\frac{M}{M + 220}},\qquad z = \text{seat},
$$

with $M$ the memory in MiB. Since $x$ and $w$ have the same scale (30 CPU-seconds, 30 seconds per unit), $dx/dw = $ CPU use in cores. The metric on the $(w, x)$ plane is $dw^2 - dx^2$; a boost with rapidity $\eta$ and $\beta = \tanh\eta$ is

$$
\begin{pmatrix} x' \\ w' \end{pmatrix} = \begin{pmatrix}\cosh\eta & -\sinh\eta\\ -\sinh\eta & \cosh\eta\end{pmatrix}\begin{pmatrix} x \\ w \end{pmatrix}
$$

(derived on the Math page). A worldline of slope $\beta$ goes vertical, $x' = $ const, in the frame with rapidity $\eta = \operatorname{artanh}\beta$, and only exists for $\lvert\beta\rvert < 1$: a process faster than one core has **no rest frame**. Rapidities add (boost by $\eta_1$, then $\eta_2$, equals one boost by $\eta_1 + \eta_2$) while velocities do not, which is why the eased boost interpolates the rapidity, not $\beta$.

The *camera* rotation is separate and Euclidean: a $4\times4$ orthogonal matrix composed of plane rotations in the XW, YW and ZW planes (rotations in four dimensions have six independent planes; the view uses three). The Schlegel projection is a central projection along $w$ from an eye at $w = 3.5$: a point $(x,y,z,w)$ goes to
$(x, y, z)\cdot 25/(3.5 - w)$. For the unrotated cube the cell at $w = +1$ is 2.5 from the eye and the cell at $w = -1$ is 4.5, so the inner cube is 5/9 the size of the outer.

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| w, outer cube to inner cube | Wall-clock time of each sample (`Snapshot::elapsed`) | $w = 1 + (t - \text{now})/30$ s: +1 now, -1 sixty seconds ago; between samples the window scrolls on by up to one sampling interval |
| x | CPU time (`cpu_time`, user plus system) | $x = -1 + (\text{CPU-s since the window opened})/30$, the start interpolated at the window's left edge (or at a later process's first sample); a falling counter counts as no growth |
| y (up) | Memory (RSS; physical footprint on macOS) | $y = -1 + 2\sqrt{M/(M + 220)}$, $M$ in MiB (`model::bounded` with knee 220) |
| z | Seat | Four bands (kernel, system, session, containers) share $-0.9$ to $0.9$ in proportion to their capacities; a newcomer takes the lowest free seat in its band; held until its worldline has left the window |
| Head sphere | A process alive in the drawn sample | At $w = +1$, last segment extended to now; radius $\propto$ perspective factor and $0.6 + 0.3(y + 1)$; colour by kind, darker the further back in $w$; a newborn grows in |
| Worldline | The process's samples in the window | At most one sample per 1 s bin; runs whose slopes differ by less than 0.02 (and memory by less than 0.002) merge, so an idle process is one segment |
| Blue segment | CPU above one core | Slope above 1.05 (the margin absorbs clock-tick quantisation): only a multithreaded process |
| Cherenkov glow and cone | A superluminal head | $\beta$ = slope over the last 5 s; 12 hairlines on a cone of half-angle $\arccos(1/\beta)$ trailing the head |
| Dashed diagonal "c = one core" | Exactly one core | The line $x = w$ on the x-w face at $y = z = +1$ |
| Fork vertex and curve | A process first seen after the first recorded sample whose parent (`ppid`) was a recorded live process | Starts at the parent's $(x, y, z)$ and smoothsteps $y$ and $z$ to its own over 2 s; a glow marks the vertex |
| Exited worldline | A process no longer listed | Kept without a head until it leaves the window; drawn only without subtree focus, newest exits first, within `--limit` |
| Cyan marks | Disk I/O (`io_rate` above 0) at a sample | A 2.4 px stretch of worldline 0.4 s either side, one per sample with I/O, at most 2000 a frame, newest first |
| Links | Sockets | The shared link code between heads; `c` cycles them |

**Decorative:** the wireframe (the now cell brighter, the past cell dimmer), the 4D orientation and its idle drift (camera only), the dim tilted cube drawn while boosted (the lab frame's now cell seen from the boosted frame), and the *display compression* described below.

## How isotop realises it

**Recording.** `Worldlines::record` runs on every sample whichever view is shown, at most every `CADENCE = 0.25` s of `elapsed`, and keeps each process's samples (`elapsed`, `cpu_time`, `memory`, whether it did I/O) for `RETAIN = 180` s, so a rewound frame still has its minute. Cumulative CPU needs its own recorder, hence its own cadence; the shared history in `history.rs` holds rates, not CPU-seconds.
Headless demo runs replay 60 s of samples first (`Phase::Replay`); a live headless run starts with its two samples, so the window is short and the legend says `window Ns`.

**Seats.** `Bands` allocates seats: a band holds a power of two of at least `BAND_SEATS = 16` seats, doubles when full and never shrinks; the first sample reserves twice the seats it fills so the exited processes, which hold their seats for a minute, and their replacements fit without a doubling. Seats are held until the worldline leaves the window. A seat is stable for life; z is never
reshuffled except by a doubling. (This is the [stable layout](Theory-Stable-Layout) rule: x is a 60 s average, y is memory, z is the seat.)

**Tracks.** `Worldlines::track` turns a line into a polyline: it interpolates growth at the window's left edge, bins to 1 s, blends a forked child onto its parent over `FORK_SECONDS = 2`, extends the head to the display time, and `merge`s collinear runs. `chord` is the slope over the visible window, `recent` the slope over the last `RECENT = 5` s; the chord decides rest frames and the recent slope decides the blue segments and the cone.

**Projection.** `Projection::world`: boost (about the pivot event $(x, w)$ of the selected head), display compression, 4D rotation, then the perspective divide with factor $1/\max(3.5 - w, 0.75)$ and scale 25 world units. Everything happens before the display list exists, so CPU/GPU parity is free and nothing changes in `shaders.wgsl`. **Display compression**: beyond 1, $x$ and $w$ are mapped by $1 + 0.6\tanh((|v| - 1)/0.6)$, so after a boost
or for a superluminal process nothing leaves the viewing frustum or blows up the divide (its maximum is 1.6 and the nearest approach to the eye is $3.5 - 2.7 = 0.8$, guarded by the 0.75 floor). It is a monotone distortion applied to endpoints, so it bends straight worldlines beyond the cube into polylines.

**Camera orientation.** `Orientation`: canonical angles $(0.12, 0.10, -0.08)$ rad in XW, YW and ZW, eased with a 0.2 s time constant, plus an idle drift of a full turn in XW every `DRIFT_PERIOD = 120` s (a function of the clock, so `--time` repeats it); any of `,` `.` `;` stops the drift; Home or `r` restores the canonical orientation, the lab frame and the drift.

**Boost.** `b` boosts into the rest frame of the selected process (pivot at its head, rapidity $\operatorname{artanh}$ of its chord slope, eased with a 0.4 s time constant); a chord slope at or above 0.999 has no rest frame and the legend says `no rest frame: name beta 2.3`. The guided tour boosts into each process it visits; one without a rest frame stays in the lab frame and its callout says `no rest frame`.
While boosted, a dim tilted copy of the lab frame's "now" cell shows that the present is no longer simultaneous.

**Picking and parity.** Heads are `Sphere` items with identities; worldlines are hairlines (`Frame::hairline`, ribbons made of triangles, so the two rasterizers agree) that are never pickable. Selecting a head lights its whole worldline and the fork vertices of its ancestors. The row sets `links: All`, `link_strokes: true`, demo options `churn: true` and `exact_cpu_time: true`.

**Degradation.** A process whose I/O counters are unreadable (other users' processes without privileges) has no I/O marks and the legend counts them (`I/O unreadable for N`). A rewound frame older than the seat release can show an exited worldline on a seat since given to a newcomer. On macOS `parent` is the *responsible* process, so forks branch from the responsible app rather than the Unix parent.

## Reading the picture

What I see in a demo render at `--time 90`: a thin grey wireframe cube in a tilted view; inside it a dense sheaf of lines running from the upper left towards the lower right in kind colours (cyan, gold, teal, lilac, slate), each a worldline of cumulative CPU; a comb of long thin lines running straight down towards the middle of the cube (idle processes: with no CPU their worldline runs along $w$ only, which the perspective turns into a line toward the vanishing point); a few pink strokes (most likely the demo's zombies, whose state colour is pink and is fixed at first sight); shaded spheres clustered towards the right front where the heads are, each with a size that grows with memory; and small starbursts of light-blue hairlines at a few heads, the Cherenkov cones of processes using more than one core.

* **Measured:** the worldlines' $x$ and $w$ (CPU and time), $y$ (memory), the I/O marks, the ancestry of forked children (as seen at the first sample in which they appear).
* **Derived:** z (the seat), the slope classes (at rest, lightlike, superluminal), the rest frame of a process, the Cherenkov angle $\arccos(1/\beta)$.
* **Decorative or distorting:** the wireframe, the drift, the compression of $x$ and $w$ beyond 1, the perspective (an object in the inner cube looks smaller because it is farther in $w$, not because it uses less memory; head radius is multiplied by the perspective factor on purpose, and shading darkens with distance in $w$).
* **Time is shared:** a quarter turn in XW makes time the horizontal axis and CPU the depth along $w$. That is the only orientation that reads like a conventional chart, and there CPU is the distance from you along a direction you cannot see.
* **The light cone is real for one thread.** A *measured* slope slightly above 1 on a single-threaded process (1.01, say) is quantisation: on Linux `/proc/<pid>/stat` reports CPU time in clock ticks (usually 10 ms), and samples are taken at slightly irregular instants. The blue threshold of 1.05 hides most of it.
* **Fork vertices are inferred.** A child appears at the first sample in which it is listed, with its parent read at that sample; a child born and exited between two samples is never seen, and a process re-parented to init has only its first parent.
* **Invisible:** anything older than 60 s (kept 180 s only for rewind), CPU bursts shorter than the 1 s binning, and any structure in $z$ beyond kind and seat.
* **A boost changes nothing physical.** It is a linear map of $(x, w)$ that preserves $dw^2 - dx^2$; for processes with $|\beta|<1$ it removes the process's own CPU rate from the picture, with a Lorentz rescaling and the tilt of simultaneity that a Galilean shear would not have.

## Try it

```sh
./target/release/isotop --demo --view tesseract --width 1600 --output /tmp/tesseract.png --renderer cpu
./target/release/isotop --demo --view tesseract --time 90 --width 1600 --output /tmp/t90.png
./target/release/isotop --view tesseract
```

Keys: `,` and `.` turn a quarter turn back and forward in the current plane, `;` cycles the plane XW, YW, ZW, `b` boosts into the selected process's rest frame (`b` again or Esc returns to the lab frame; the next Esc has its usual meaning), Home or `r` restore everything, and the usual global keys; Tab, the picker or `--view tesseract` (no number key: all ten are taken) reach the view.

![Boost and Schlegel projection: the Minkowski diagram before and after a boost, and the tesseract at four rotations in the XW plane](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/lorentz-schlegel-boost.svg)

*Top: the (x, w) plane in the lab frame and boosted into the rest frame of a 0.6-core process; bottom: the projection used by the view for XW angles of 0, the canonical 0.12 rad, 45 degrees and a quarter turn. Script: `docs/figures/lorentz-schlegel-boost.py`.*

## Where in the code

* `src/tesseract.rs`: `Worldlines` (`record`, `track`, `curve`, `x_at`, `origin`, `exited`), `Bands`, `Seat`, `Window`, `Track`, `merge`, `slope`, `rotation`, `rotate`, `plane_rotation`, `boost`, `rest_rapidity`, `compress`, `memory_axis`, `Orientation`, `Boost`, `Projection`, `Tesseract` (`draw`, `steer`, `draw_frame`, `key`), `describe_slope`; constants `WINDOW`, `RETAIN`, `CADENCE`, `UNIT`, `BIN`, `RECENT`, `EYE`, `NEAREST`, `WORLD`, `CANONICAL`, `COMPRESSION`, `SUPERLUMINAL`, `NO_REST_FRAME`.
* `src/views.rs`: the `tesseract` row (keys string `, . quarter turn | ; plane | b boost`, `links: All`, `link_strokes: true`, replay warm-up, demo options). `src/render.rs`: `Frame::hairline`, `Stage::toured`, `callout`. `src/status.rs`: `unreadable_note`.
* Tests: `a_process_using_one_core_follows_the_light_cone`, `single_threaded_processes_never_leave_the_light_cone`, `rotations_preserve_lengths`, `a_quarter_turn_in_xw_swaps_time_and_cpu`, `boosts_preserve_the_interval`, `boosting_into_a_process_leaves_it_at_rest`, `no_rest_frame_exists_for_a_superluminal_process`, `children_branch_from_their_parent_worldline`, `the_tour_boosts_into_a_rest_frame_or_says_there_is_none`.
* Design: `docs/briefs/views-2026-10.md` (section 18, `tesseract`), including the rejected alternatives (a Q4 hypercube of boolean process states and an Asimov grand tour). Views are compile-time modules; there is no plugin system.

## Variations and open questions

1. **A time-like y.** Replace memory by an I/O rate so the cube's three space axes are CPU, I/O and seat; collapse and sparsity (most processes do no I/O) were the reasons the design rejected it.
2. **Proper time.** Print each worldline's proper time $\Delta\tau = \Delta w\sqrt{1 - \beta^2}$: "how long the process spent not using CPU, in the Minkowski sense". Is it a useful number or only a curiosity?
3. **Superluminal as a ratio.** Cherenkov in a medium has $\cos\theta = 1/(n\beta)$; the view uses $n = 1$. Give a cgroup with a CPU quota a "refractive index".
4. **Chrono-boost.** Ease the rapidity of the *tour* by the rapidity difference between consecutive targets so the tour's motion is a composition of boosts.
5. **Inverse problem.** Given a screenshot at a known orientation, recover the 4D rotation; it is an exercise in projective geometry and a test of whether the view is invertible.
6. **Rotation in all six planes.** The view composes XW, YW and ZW (those that mix time); XY, XZ and YZ are space rotations that the 3D camera already provides.

## References

* A. Einstein, "Zur Elektrodynamik bewegter Korper", *Annalen der Physik* 17 (1905), 891-921.
* H. Minkowski, "Raum und Zeit", *Physikalische Zeitschrift* 10 (1909), 104-111 (lecture of 1908).
* C. H. Hinton, *A New Era of Thought* (London, 1888), which introduced the name tesseract.
* V. Schlegel, the projection models of regular four-dimensional bodies (1886; bibliographic details unverified).
* H. S. M. Coxeter, *Regular Polytopes* (3rd ed., Dover, 1973).
* P. A. Cherenkov's radiation, as summarised in J. D. Jackson, *Classical Electrodynamics* (3rd ed., Wiley, 1999), chapter 13.
* [Math Lorentz and Schlegel](Math-Lorentz-and-Schlegel), [Theory History](Theory-History), [Theory Camera and Projection](Theory-Camera-and-Projection), [Theory Stable Layout](Theory-Stable-Layout).
