# Tesseract: worldlines in four dimensions

**View:** `tesseract` · **Code:** `src/tesseract.rs` · **Prototype:** `docs/mockups/code/tesseract.py`

## What you are looking at

Each process is a **worldline**: a curve through four axes over the last 60 seconds.

| Axis | Measured quantity | Scale |
|---|---|---|
| $x$ | CPU-seconds the process has used since the window opened (a child forked inside the window starts at its parent's $x$ at the moment of the fork, so only the slope $dx/dw$ is purely the child's own CPU use) | 30 CPU-seconds per unit |
| $y$ | memory, on a bounded scale (`model::bounded`) | half-scale at 220 MiB |
| $z$ | the process's seat: the band of its kind and its place in it | stable per process |
| $w$ | time, from $-1$ a minute ago to $+1$ now | 30 seconds per unit |

Because $x$ and $w$ use the same scale (30 and 30), the **slope** of a worldline $dx/dw$ is CPU-seconds per second, which is the number of cores in use. One core is slope 1. A process that does not run has slope 0. A fork is a vertex where one worldline becomes two.

The view then does three things to the picture: projects it from 4D to 3D, optionally rotates it in 4D, and optionally applies a **Lorentz boost** that shears worldlines the way a change of reference frame would in special relativity.

## History

**A fourth dimension.** Mathematicians had long treated four coordinates as algebra, but the *geometry* arrived in the 1850s with Ludwig Schläfli, who found the regular polytopes of any dimension: in 4D, six of them, including the 4-cube. The English mathematician Charles Howard Hinton popularized the idea; in his 1888 book *A New Era of Thought* he coined the word **tesseract**, from the Greek for "four rays", the four edges that meet at each vertex. Hinton taught his readers to visualize 4D with sets of coloured cubes. Victor Schlegel's diagrams of polytopes, projected so that one cell contains the rest, date from the 1880s.

**Time as the fourth.** Hendrik Lorentz (1904) found the transformations under which Maxwell's equations keep their form, and Einstein (1905) derived them from the principle of relativity and the constancy of the speed of light. In 1908 Hermann Minkowski, Einstein's former teacher, reframed the theory as geometry: "Henceforth space by itself, and time by itself, are doomed to fade away into mere shadows, and only a kind of union of the two will preserve an independent reality." Alfred Robb (1911) introduced **rapidity** as the natural parameter of a boost. Salvador Dalí painted the cube-cross unfolded tesseract in *Corpus Hypercubus* (1954).

**Cherenkov.** Pavel Cherenkov observed a faint blue glow from radioactive sources in liquids in 1934. Ilya Frank and Igor Tamm explained it in 1937 as radiation from particles faster than light *in that medium*, and the three shared the 1958 Nobel Prize. The cone's angle is the signature of the speed.

## Background theory

### A tesseract and its shadow

A tesseract (4-cube) has 16 vertices, 32 edges, 24 square faces and 8 cubical cells. We cannot see 4D, so we project, as we project a 3D cube onto a 2D screen. Two projections matter:

- **Parallel:** drop one coordinate, yielding a cube whose edges are shifted copies of each other (what a shadow of a rotating cube looks like).
- **Perspective (Schlegel):** put an eye on the fourth axis at $w = d$ and divide the other coordinates by the distance to it. The result is the familiar cube-within-a-cube, where the cell nearest the eye is the large outer cube and the farthest cell is the small inner cube, with the other six cells squashed between them.

isotop uses the perspective form. A point $(x, y, z, w)$ lands at

$$\big(x, z, y\big) \cdot \frac{W}{d - w}, \qquad d = 3.5$$

with $W = 25$ world units per projected unit, the divisor floored at 0.75 so points near the eye do not blow up. Memory $y$ is drawn upward. Points with larger $w$ (the present) are nearer the eye and look larger and brighter; the past recedes. Worldline shade runs from about a third at $w = -1$ to full at $w = +1$.

Coordinates beyond $\pm 1$ are compressed smoothly rather than clipped: $|c| \to 1 + 0.6\tanh((|c|-1)/0.6)$, so a process that burns far more CPU than the window expected still fits on screen.

### Worked numbers

**Perspective.** With the eye at $w = 3.5$ and $W = 25$, a point at the present ($w = +1$) is magnified by $25/2.5 = 10$ world units per unit and one a minute ago ($w = -1$) by $25/4.5 = 5.6$. So the present is 1.8 times larger than the past on screen, and its cube is the outer one.

**Compression.** A coordinate of 2 is displayed at $1 + 0.6\tanh(1/0.6) \approx 1.56$. A very large one tends to 1.6.

**A boost.** For a process using half a core, $\beta = 0.5$ and the rapidity is $\eta = \operatorname{artanh}(0.5) = 0.549$, so $\cosh\eta = 1.155$ and $\sinh\eta = 0.577$. The same worldline that leaned at slope 0.5 is now vertical. A process of slope 0.9 has $\eta = 1.47$, $\gamma = \cosh\eta = 2.29$, so its rest frame is a much more violent shear. Rapidities add: boosting by 0.549 and then by another 0.549 is the same as one boost of 1.099, which is the rest frame of a process of slope $\tanh(1.099) = 0.8$, not 1.0. This is the relativistic velocity addition rule, $(0.5 + 0.5)/(1 + 0.25) = 0.8$.

**Cherenkov angles.** $\beta = 1.05$ gives $\arccos(1/1.05) = 17.8^\circ$ and $\beta = 2$ gives $60^\circ$. The cone opens as the process uses more cores.

### Rotations in four dimensions

In 3D you rotate around an axis; in 4D you rotate *in a plane*, and there are six coordinate planes ($\binom{4}{2}$). Rotating in the $xy$ plane leaves $z$ and $w$ fixed, and so on. A 4D rotation in a plane is the usual matrix

$$\begin{pmatrix} \cos\theta & -\sin\theta \\ \sin\theta & \cos\theta \end{pmatrix}$$

embedded in the identity. The view uses the three planes that involve $w$, XW, YW and ZW, because rotations among $x$, $y$ and $z$ alone are just the ordinary 3D camera. The matrices are multiplied in a fixed order, and they do **not** commute, so the order matters. Only the camera rotates in 4D. Nothing in the data is changed by it.

### Lorentz boosts

In special relativity, a boost to a frame moving at speed $\beta = v/c$ along $x$ mixes space and time with a **hyperbolic rotation** in the $(x, t)$ plane:

$$x' = x\cosh\eta - ct\,\sinh\eta, \qquad ct' = ct\cosh\eta - x\sinh\eta, \qquad \tanh\eta = \beta$$

$\eta$ is the *rapidity*. It adds under successive boosts, unlike velocities. The boost preserves $(c\,\Delta t)^2 - \Delta x^2$ (the interval), where an ordinary rotation preserves $\Delta x^2 + \Delta y^2$. That is the whole difference between space and time in the geometry.

Applied here, with $w$ standing in for $ct$ and CPU-seconds for $x$:

- A worldline with slope $\beta < 1$ is **timelike**, and there is a boost with $\eta = \operatorname{artanh}(\beta)$ in which it is vertical: the **rest frame** of that process. In that frame the process "isn't running" and the *rest of the machine* streams past at $-\beta$. This is how the view lets you look at the system from one process's point of view. The rest frame of a process at 0.5 cores is a tilt of the whole scene.
- A slope of exactly 1 (one core) is **lightlike**. No boost turns it vertical. It has no rest frame; slopes at or above 0.999 are refused (`NO_REST_FRAME`).
- A slope above 1, only possible for a **multithreaded** process using more than one core, is **superluminal** in this analogy. It has no rest frame either.

### Cherenkov radiation

A charged particle going faster than light *in a medium* radiates a cone of light at angle $\cos\theta = 1/(n\beta)$ (blue light in reactor water). The analogy here is $c$ equals one core: a process with slope $\beta$ above 1.05 (`SUPERLUMINAL`, a margin so that a single saturated thread reading 1.0 to 1.05 does not trigger it) gets a cone at $\theta = \arccos(1/\beta)$, drawn in blue as 12 beams, and the inspector gives the angle. Between 1.0 and 1.05 the worldline is lightlike and has no cone. It is a metaphor with a formula attached, not a physical identity.

## How isotop implements it

1. **Recording.** `Worldlines` samples cumulative CPU time at most every 0.25 s and keeps 180 s, in every view, so switching to Tesseract does not start with an empty minute. Seats come from `Bands`: four bands (one per kind) share $z \in [-0.9, 0.9]$ in proportion to capacity. A newcomer takes the lowest free seat; a band that fills doubles and never shrinks. Exited processes hold their seat for a minute, so a respawn does not displace a live neighbour.
2. **Drawing.** The drawn curve keeps one sample per second and merges consecutive segments whose slopes agree to within 0.02 and which share memory and seat (`merge`), so a flat process is one segment and not sixty. Forks curve from the parent's worldline to the child's seat over 2 s.
3. **Marks.** Head glow, the recent slope (taken over 5 s), I/O ticks (up to 2,000 per frame) and the cone all come from the same worldline data.
4. **Boost.** `steer` targets the rest frame of the process that was selected when you pressed `b` (selecting alone leaves the lab frame; `b` again or Escape drops the boost), or else the one the guided tour visits, easing the rapidity over 0.4 s. The boost pivots on the head of the target, so it stays fixed on screen while the rest of the world shears around it.

## Faithful, stylized, invented

- **Faithful:** the Lorentz boost matrix, rapidity addition, invariance of $\Delta w^2 - \Delta x^2$, the rest-frame condition $\eta = \operatorname{artanh}\beta$, the Cherenkov angle formula, the Schlegel perspective divide.
- **Stylized:** $c$ = one core. Nothing physical forbids a process from exceeding it, but a single thread cannot, which makes a bright line between single- and multithreaded workloads worth drawing. The display compression is a visual device.
- **Invented:** the entire spacetime identification. CPU-seconds is not a position, there is no causal structure in $(x, w)$, and a Lorentz boost of process data has no *physical* meaning. What it has is *geometric* meaning: it is a shear that makes a chosen process look stationary, which is exactly what a change of reference frame does to any line. The view is a way to ask "what does the system look like relative to this process?"
- **Approximate:** the 3D view of a 4D scene hides what it hides: two worldlines can look to cross in the projection and miss each other in 4D.

## Reading the picture

- Vertical (in the boost frame) lines are the reference frame's own process. Everything else leans.
- A line that hugs the lightlike diagonal is a single thread saturating one core.
- A line steeper than that is multithreaded, and it carries the cone.
- Fork vertices where lines spread are the process tree in time.
- If a process has no rest frame (the inspector says so), selecting it leaves the view unboosted.

## Further reading

- Coxeter, H. S. M., *Regular Polytopes* (1973), on the 4-cube and its projections.
- Taylor & Wheeler, *Spacetime Physics* (1992), on the interval, boosts and rapidity.
- Minkowski, H. (1908), "Raum und Zeit": the original statement that space and time are one geometry.
- Banchoff, T. F., *Beyond the Third Dimension* (1990) for the Schlegel viewpoint.
