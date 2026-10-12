# Lorentz and Schlegel

Two pieces of geometry make the [Tesseract](View-Tesseract) work, and they are different kinds of thing. A **Lorentz boost** is a hyperbolic rotation of the time-and-space plane that preserves the Minkowski interval; it is physics (here, a re-description of
the data). A **Schlegel projection** is a perspective projection of a four-dimensional body into three dimensions; it is a camera. Four-dimensional *Euclidean* rotations sit between them and are the source of most of the visual drama. This page derives each from first principles, counts the
parts of the tesseract, shows what the projection does to it at several rotations, and says which of the view's pictures is which.

![Top: the (x, w) plane in the lab frame, boosted, and its invariant hyperbolas. Bottom: the tesseract's Schlegel projection at four rotations in the XW plane](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/lorentz-schlegel-boost.svg)

*Top row: worldlines $x = -1 + \beta(w + 1)$ before and after the boost into the rest frame of the 0.6-core process (it goes vertical, the "now" line tilts, the light cone stays), and the hyperbolas of constant interval along which a boost moves an event. Bottom row: the 32 edges of the tesseract
with the view's projection at XW angles 0, 0.12 (canonical), 45 and 90 degrees. Script: `docs/figures/lorentz-schlegel-boost.py`.*

## Theory

### 1. Minkowski space

Take coordinates $(w, x)$, with $w$ the time (here in units where the speed of light is 1, so that $x$ and $w$ have the same scale) and $x$ one space axis; $y, z$ come along unchanged. The *interval* between two events is

$$
\Delta s^2 = \Delta w^2 - \Delta x^2 .
$$

It is positive for *timelike* separations (one event can influence the other), negative for *spacelike*, zero for *lightlike*: the two lines $x = \pm w$ through an event form its *light cone*. A particle's history is a **worldline**. If it never exceeds the speed of light its worldline always lies inside every light cone on it, with slope $\lvert dx/dw\rvert < 1$. Along it the
*proper time* is

$$
d\tau = \sqrt{dw^2 - dx^2} = dw\sqrt{1 - \beta^2},\qquad \beta = \frac{dx}{dw}.
$$

### 2. The boost, derived

We want the linear maps of the $(w, x)$ plane that keep $\Delta s^2$ unchanged and fix the origin, and that reduce to the identity continuously. Euclidean rotations preserve $w^2 + x^2$ and have the form $(\cos\theta, \sin\theta)$ because $\cos^2 + \sin^2 = 1$. The Minkowski form has a minus sign, and $\cosh^2\eta - \sinh^2\eta = 1$, so write

$$
\begin{pmatrix} w' \\ x' \end{pmatrix} =
\begin{pmatrix}\cosh\eta & -\sinh\eta\\ -\sinh\eta & \cosh\eta\end{pmatrix}
\begin{pmatrix} w \\ x \end{pmatrix}.
$$

Check: $w'^2 - x'^2 = (w\cosh\eta - x\sinh\eta)^2 - (x\cosh\eta - w\sinh\eta)^2 = (w^2 - x^2)(\cosh^2\eta - \sinh^2\eta) = w^2 - x^2$. These are all the continuous ones (the orthogonal group of the Minkowski form, $SO^+(1,1)$, is one-dimensional: the parameter $\eta$ is the **rapidity**, a hyperbolic
angle). The images of any point trace out a hyperbola $w^2 - x^2 = \text{const}$ (the figure's right panel), exactly as Euclidean rotations trace circles.

*Interpretation.* The new frame moves with velocity $\beta = \tanh\eta$ relative to the old: a worldline of slope $\beta$ satisfies $dx' = \cosh\eta\,(dx - \beta\,dw)$, which is zero. With $\gamma = \cosh\eta = 1/\sqrt{1-\beta^2}$ and $\gamma\beta = \sinh\eta$:

$$
x' = \gamma(x - \beta w),\qquad w' = \gamma(w - \beta x).
$$

This is the Lorentz transformation (Einstein 1905, in the geometric form Minkowski gave it in 1908). Three consequences:

* **Rapidities add.** Two boosts compose to a boost with $\eta = \eta_1 + \eta_2$, because hyperbolic rotations commute and add angles like rotations do. Velocities do not: $\tanh(\eta_1 + \eta_2) = (\beta_1 + \beta_2)/(1 + \beta_1\beta_2)$. So anything that must *ease* between frames should interpolate rapidity: the view does.
* **Simultaneity tilts.** The surfaces $w' = \text{const}$ are lines $w = \text{const} + \beta x$ in the old frame. The "now" slice of one frame is not the "now" slice of another, which is why the view draws a dim tilted copy of the lab's now cell while boosted.
* **No rest frame at or beyond the light cone.** $\beta = \tanh\eta$ has $\lvert\beta\rvert < 1$ always; a worldline of slope $\ge 1$ cannot be made vertical. The view treats a chord slope of 0.999 or more as having no rest frame.

*What the boost does to a worldline $x = \beta_0 w$:* its slope becomes $(\beta_0 - \beta)/(1 - \beta\beta_0)$ (relativistic velocity subtraction), its proper time is unchanged, and the picture of all other worldlines is sheared and rescaled around the pivot event.

### 3. Rotations in four Euclidean dimensions

A rotation of $\mathbb R^n$ is a linear map preserving lengths and orientation: $R^T R = I$, $\det R = 1$. In 4D the group $SO(4)$ has dimension $\binom 42 = 6$: one *plane rotation* for each pair of axes (XY, XZ, YZ, XW, YW, ZW), each fixing the two other axes pointwise.
The XW rotation by angle $\theta$ is

$$
R_{XW}(\theta) = \begin{pmatrix}\cos\theta & 0 & 0 & -\sin\theta\\ 0&1&0&0\\0&0&1&0\\ \sin\theta & 0&0&\cos\theta\end{pmatrix}.
$$

Rotations in planes that share an axis do *not* commute, so the order of composition matters; the view composes $R = R_{XW}R_{YW}R_{ZW}$ (applied right to left). A generic element of $SO(4)$ is a *double rotation*: it turns two orthogonal planes by independent angles, $\theta_1$ and $\theta_2$, and fixes no axis. Algebraically $SO(4) \cong (SU(2)\times SU(2))/\mathbb Z_2$, acting on a point as a
quaternion $q$ by $q \mapsto a\,q\,\bar b$ for unit quaternions $a, b$; the *isoclinic* rotations (equal angles) are those with $a$ or $b$ trivial. A quarter turn in XW sends the $x$ axis to the $w$ axis and the $w$ axis to $-x$ (the view's test `a_quarter_turn_in_xw_swaps_time_and_cpu`).

**Boost versus rotation.** In the same $(x, w)$ plane the Euclidean rotation uses $\cos, \sin$ (circles; compact; $2\pi$-periodic) and the boost uses $\cosh, \sinh$ (hyperbolas; non-compact). The first changes how *you look*; the second changes the *frame the data are described in* and preserves the interval instead of the length. That is the split the view keeps: rotations are camera only,
boosts are physics.

### 4. The tesseract

The tesseract (8-cell, hypercube, 4-cube) is the product $[-1, 1]^4$. Its faces of each dimension are choices of which coordinates are fixed at $\pm1$:

| Faces | Count | Formula |
|---|---|---|
| Vertices | 16 | $2^4$ |
| Edges | 32 | $4\cdot 2^3$ |
| Square faces | 24 | $\binom 42 2^2$ |
| Cubic cells | 8 | $4\cdot 2$ |

and Euler's characteristic $16 - 32 + 24 - 8 = 0$, that of the 3-sphere which is the boundary of a 4-ball. The view draws the 32 edges: for each of the 16 corners and each axis, the edge to the corner with that bit set.

### 5. Projection from four to three dimensions

**Perspective along $w$.** Put the eye at $E = (0, 0, 0, e)$ and project onto the hyperplane $w = 0$. The ray from $E$ through a point $P = (\mathbf r, w)$ reaches $w = 0$ at parameter $\lambda$ solving $e + \lambda(w - e) = 0$, so $\lambda = e/(e - w)$ and the image is

$$
\mathbf r' = \frac{e}{e - w}\,\mathbf r .
$$

The view uses $\mathbf r\,\cdot 25/(3.5 - w)$, which is this with $e = 3.5$ and a uniform scale of $25/e$. The unrotated tesseract then shows its $w = +1$ cell at perspective factor $1/(e - 1) = 0.4$ and its $w = -1$ cell at $1/(e+1) = 0.222$: **a cube inside a cube**, the inner 5/9 the size of the outer
(in general $(e-1)/(e+1)$). The six remaining cells ($x = \pm1$, $y = \pm1$, $z = \pm1$) are the truncated pyramids joining them. This is the classical *Schlegel diagram* of the tesseract (Schlegel, 1886): the eye sits just outside one cell, the projection maps that cell to the outer boundary and everything else lies inside it.
Because a central projection maps straight lines to straight lines, edges and straight worldline segments stay straight; the combinatorics (which vertices are joined) is preserved.

**The alternative.** *Stereographic* projection of the circumscribed 3-sphere from a pole is conformal (angles are preserved) and maps the tesseract to curved cells; it is the usual "bulging" picture. The view uses the perspective (projective) one because its straight lines are straight and because the whole computation then happens before the display list exists.

**Rotating first.** The view rotates the 4D points and *then* divides by $e - w'$ with $w'$ the rotated $w$-coordinate. A rotation in XW by angle $\theta$ moves the cell which is nearest the eye from $w = +1$ towards $x = \pm1$; at a quarter turn the outer cube is the $x = +1$ cell, and by a half turn $w \to -w$ the cube-in-cube is turned *inside out*:
the old inner cube is the outer one. The singularity at $w' = e$ cannot be reached by the rotated unit cube (its maximum $w'$ is 2 for the corners), and the view adds two safeguards, below.

**Display compression.** After a boost, or for a superluminal worldline, $x$ and $w$ can exceed 1. The view first compresses both with

$$
c(v) = \begin{cases} v & \lvert v\rvert \le 1\\ \operatorname{sgn}(v)\bigl(1 + 0.6\tanh\tfrac{\lvert v\rvert - 1}{0.6}\bigr) & \lvert v\rvert > 1\end{cases}
$$

which is monotone, has continuous first derivative at $\lvert v\rvert = 1$ (the slope of the tanh branch there is 1) and is bounded by 1.6. Then the largest rotated $\lvert w'\rvert$ is at most $\sqrt{1.6^2 + 1.6^2 + 1 + 1} = 2.67$ and $e - w' \ge 0.83 > 0.75 =$ `NEAREST`, the floor on the divisor. The price is a distortion: beyond the cube, distances are not to scale, and straight worldlines
bend where compression applies (the view compresses endpoints and draws straight hairlines between them).

### 6. Cherenkov radiation, as theory

A charged particle moving through a medium of refractive index $n$ faster than the phase speed of light, $\beta > 1/n$, emits a cone of light of half-angle $\cos\theta_C = 1/(n\beta)$ (Cherenkov, 1934; theory by Frank and Tamm, 1937). In vacuum $n = 1$ and no particle exceeds $c$, so there is no such radiation; the view
uses $n = 1$ and $\beta$ in cores, which is a *metaphor*: there is nothing "superluminal" about a process using two cores, only about the arithmetic of its CPU-time curve. The angle shown is $\arccos(1/\beta)$: $\beta = 2.3$ gives $64^\circ$.

## What the code does

The functions are short and mirror the derivations above. `boost(point, pivot, rapidity)` returns `[pivot_x + dx cosh - dw sinh, y, z, pivot_w + dw cosh - dx sinh]`, with $(dx, dw)$ the offset from the pivot: the boost of section 2 about a chosen event (the head of the selected process), leaving $y$ and $z$ alone. `rest_rapidity(beta)` is $\operatorname{artanh}\beta$ for $\lvert\beta\rvert < 0.999$ and `None`
otherwise. `plane_rotation(a, b, angle)` builds the matrices of section 3 (`matrix[a][a] = cos`, `matrix[a][b] = -sin`, `matrix[b][a] = sin`, `matrix[b][b] = cos`); `rotation(angles)` multiplies the XW, YW and ZW ones; the canonical angles are $(0.12, 0.10, -0.08)$ rad, eased with a 0.2 s time constant, with a drift of a full turn in XW every 120 s.
`Projection::world` applies the boost, then `compress`, then `rotate`, then the divide `1 / max(EYE - w, NEAREST)` with `EYE = 3.5` and `WORLD = 25`; memory is up. The frame (`draw_frame`) draws the 32 edges of the *lab-frame* tesseract through the same projection (so it is boosted along with the data), plus the dashed light-cone diagonal $[v, 1, 1, v]$ of the $y = z = 1$ face,
and `draw_lab_now` draws the unboosted "now" cell dimly while the rapidity is non-zero.

Tests that pin the mathematics: `rotations_preserve_lengths`, `a_quarter_turn_in_xw_swaps_time_and_cpu`, `boosts_preserve_the_interval`, `boosting_into_a_process_leaves_it_at_rest`, `no_rest_frame_exists_for_a_superluminal_process`.

## Reading the picture

* **Theory vs code.** Everything in sections 1 to 5 is standard mathematics; the code is a direct transcription with three engineering additions (display compression, the `NEAREST` floor, easing).
* **A boost applied to measurements.** The "frame" of a process has no physical content: a boost mixes wall-clock time and CPU time, which nothing in the kernel does. What it *does* give is a useful relative description: the chosen process stands still, every other process's slope is its CPU rate *relative* to it (relativistic subtraction), and
  the now-slice tilts. It preserves the interval $\Delta w^2 - \Delta x^2$ of any pair of events: for a single process, $\Delta w^2(1 - u^2)$ with $u$ its CPU use in cores, the squared proper time.
* **Rotation versus boost.** If the picture changes when you press `,` it is a rotation (camera) and lengths in 4D are preserved; if it changes when you press `b` it is a boost, and 4D lengths are *not* preserved but the interval is.
* **Perspective makes the past small.** An object in the inner cube looks smaller because it is farther in $w$ along the viewing direction; the view's head radius multiplies by that factor deliberately, and brightness also falls with $w$.
* **What cannot be read.** Absolute CPU from the picture alone beyond slope (the cube is unit-free until you read the scale: 30 CPU-seconds per unit), and anything about the fourth axis except through perspective cues; a quarter turn in XW is the way to read $w$ as a horizontal axis.

## Try it

```sh
./target/release/isotop --demo --view tesseract --width 1600 --output /tmp/t.png --renderer cpu
python3 docs/figures/lorentz-schlegel-boost.py        # the figure above
```

In the live view press `;` to select the plane (the legend names it), then `,` or `.` for quarter turns; `b` with a process selected boosts into its rest frame.

## Where in the code

* `src/tesseract.rs`: `boost`, `rest_rapidity`, `plane_rotation`, `multiply`, `rotation`, `rotate`, `compress`, `Projection`, `Orientation`, `Boost`, `Tesseract::steer`, `draw_frame`, `draw_lab_now`; constants `EYE`, `NEAREST`, `WORLD`, `CANONICAL`, `COMPRESSION`, `NO_REST_FRAME`, `BOOST_EASE`, `TURN_EASE`, `DRIFT_PERIOD`.
* Figure: `docs/figures/lorentz-schlegel-boost.py` (re-implements `boost`, `compress`, `rotation` and the projection in Python).

## Variations and open questions

1. **Stereographic mode.** Offer the conformal projection as a second camera for the wireframe (the worldlines would then need curved segments).
2. **Boost in two planes.** The view boosts only in $(w, x)$; a boost in $(w, y)$ would mix time with memory, which has no counterpart in the data. Is there an observable that is boost-covariant in $(w, z)$?
3. **Double rotation.** Idle drift is a simple rotation in XW; a slow isoclinic rotation would never return the picture to a recognisable alignment. Is it nicer?
4. **Proper time as a column.** Show $d\tau$ in the inspector as "time not spent running" for single-threaded processes.
5. **Hyperbolic camera.** A boost with rapidity proportional to the *average* CPU of the whole machine would put a typical process at rest: a natural "centre-of-momentum frame" of the system.

## References

* A. Einstein, "Zur Elektrodynamik bewegter Korper", *Annalen der Physik* 17 (1905), 891-921.
* H. Minkowski, "Raum und Zeit", *Physikalische Zeitschrift* 10 (1909), 104-111.
* V. Schlegel, on projection models of the regular four-dimensional bodies (1886; exact title and venue unverified here).
* C. H. Hinton, *A New Era of Thought* (1888).
* H. S. M. Coxeter, *Regular Polytopes* (3rd ed., Dover, 1973).
* P. A. Cherenkov (1934); I. M. Frank and I. E. Tamm, *Comptes Rendus (Doklady) de l'Academie des Sciences de l'URSS* 14 (1937), 109-114 (details as recalled; unverified).
* J. H. Conway and D. A. Smith, *On Quaternions and Octonions* (A K Peters, 2003), on $SO(4)$ and quaternions.
* [View Tesseract](View-Tesseract), [Theory Camera and Projection](Theory-Camera-and-Projection), [Theory Rasterization](Theory-Rasterization).
