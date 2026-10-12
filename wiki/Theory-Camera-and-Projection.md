# Theory Camera and Projection

isotop's camera has no perspective. Every world point goes through one affine map, a rotation about the vertical axis, a tilt, a uniform scale and a translation, and nothing is ever divided by distance. The result is an axonometric view, isometric by default, in which parallel lines stay parallel, a body is the same size near and far, and the depth that decides who covers whom is a plain linear function of the world coordinates. This page derives that map from the code, shows why the constant `SCALE` is the square root of 3/2, derives its inverse (which is what panning and zooming need), and then covers the layer above it: how the camera eases towards a goal, how it fits a view into the frame, and how the tour steers it.

![The three cubes of the camera at LOWEST_PITCH, ISOMETRIC and top-down, and foreshortening against pitch](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/theory-isometric-axes.svg)

## Theory: orthographic and axonometric projection

A *parallel* projection maps points to an image plane along a fixed direction; it is *orthographic* when that direction is perpendicular to the plane, which is the case here because depth is just the third coordinate of a rotation. An *axonometric* projection is an orthographic one whose image plane is oblique to the coordinate axes, so all three axes show; it is *isometric* when the three axes are foreshortened equally.

Take a unit viewing vector $V$ (from the scene towards the viewer). An axis $e_i$ projects to a segment of length $\sqrt{1 - (e_i\cdot V)^2}$. Equal foreshortening needs $V_x^2 = V_y^2 = V_z^2 = \tfrac13$, so $V = (1,1,1)/\sqrt3$ up to signs, and every axis shrinks by $\sqrt{2/3} = 0.8165$. The elevation of that viewing direction above the ground plane is $\arcsin(1/\sqrt3) = 0.615480$ rad $= 35.264^\circ$, the "true isometric" angle that isotop calls `ISOMETRIC`. A uniform scale of $\sqrt{3/2} = 1.224745$ (`SCALE`) then restores each axis to length 1.

Because there is no perspective divide, the homogeneous coordinate $w$ stays 1, screen-space interpolation of any attribute is already exact (see [Theory Rasterization](Theory-Rasterization)), and depth is affine ([Theory GPU and Parity](Theory-GPU-and-Parity) shows what the GPU does with it).

## The code: Camera::view and Frame::project

```text
pub struct Camera {
    center: [f32; 2],   // ground point at the view centre
    zoom: f32,          // pixels per screen unit, default 5.0, clamped to 0.3..65
    rotation: f32,      // turn about the vertical axis, default 0
    pitch: f32,         // elevation above the ground: default ISOMETRIC, 0.35 ..= pi/2
    globe_turn, globe_tilt, globe_held   // Globe only
}
```

Constants (`render.rs`): `SCALE = 1.224745`, `ISOMETRIC = 0.615480`, `LOWEST_PITCH = 0.35` (20.05 degrees: the camera never looks from below the ground), `ORIGIN_Y = 0.57`. Write $\theta = \text{rotation} + \pi/4$, $s = \sin\theta$, $c = \cos\theta$, $\varphi = \text{pitch}$, and $(\Delta x, \Delta y) = (x - c_x,\; y - c_y)$ for the ground offset from the camera centre. `Camera::view` computes

$$
\text{across} = \Delta x\,c - \Delta y\,s, \qquad \text{along} = \Delta x\,s + \Delta y\,c,
$$

$$
\begin{aligned}
u &= \text{SCALE}\cdot \text{across}\\
v &= \text{SCALE}\cdot(\text{along}\,\sin\varphi - z\cos\varphi)\\
D &= \text{along}\,\cos\varphi + z\,\sin\varphi
\end{aligned}
$$

and `Frame::project` turns that into frame pixels, with the view centred horizontally and `ORIGIN_Y` of the way down the frame (the centre sits a little below the middle so buildings have headroom):

$$
X = \tfrac12 W + \text{zoom}\cdot u, \qquad Y = 0.57\,H + \text{zoom}\cdot v, \qquad \text{depth} = D .
$$

As one matrix acting on $(\Delta x, \Delta y, z)$, with the image-plane rows scaled by $\text{zoom}\cdot\text{SCALE}$:

$$
\begin{pmatrix} X - W/2 \\ Y - 0.57H \\ D \end{pmatrix}
=
\begin{pmatrix}
\kappa c & -\kappa s & 0\\
\kappa s\sin\varphi & \kappa c\sin\varphi & -\kappa\cos\varphi\\
s\cos\varphi & c\cos\varphi & \sin\varphi
\end{pmatrix}
\begin{pmatrix}\Delta x\\ \Delta y\\ z\end{pmatrix},
\qquad \kappa = \text{zoom}\cdot\text{SCALE}.
$$

Properties, each checkable by hand (and checked numerically while writing this page):

* The three rows, divided by $\kappa$ for the first two, are mutually orthogonal unit vectors, and the determinant of that rotation is $+1$. So the map is a rigid rotation into (right, down, towards-viewer) coordinates followed by a uniform scale of the image plane by $\kappa$ and *no* scale of depth. Depth is therefore measured in world units and is the same whatever the zoom: `Item::Sphere`'s `depth` field is a world radius for this reason.
* The third row is the viewer vector $V = (s\cos\varphi,\; c\cos\varphi,\; \sin\varphi)$ that `Camera::viewer` returns, and $D = (p - \text{centre})\cdot V$. Larger $D$ is nearer, which is why the rasterizers keep the fragment with the larger depth.
* At rotation 0 and $\varphi = \text{ISOMETRIC}$, $s = c = 1/\sqrt2$, $\sin\varphi = 1/\sqrt3$, $\cos\varphi = \sqrt{2/3}$, so the world axes land at $x \mapsto (\tfrac{\sqrt3}{2}, \tfrac12)$, $y \mapsto (-\tfrac{\sqrt3}{2}, \tfrac12)$, $z \mapsto (0, -1)$ screen units (down is positive $Y$): all of length one, the ground axes at 30 degrees below the horizontal, and every axis with depth component $1/\sqrt3$. A unit test (`default_camera_is_isometric_and_ground_inverts_the_projection`) asserts the first two numbers.
* **Handedness.** The screen triple (right, down, towards the viewer) is left-handed, and the map has determinant $+1$, so the displayed world $(x, y, z\text{ up})$ is a left-handed set. Read as a right-handed physical frame, the picture is the mirror image of what a real camera would see. This is derived, not documented in the source; it matters for any view that draws a real-world orientation.

### Foreshortening against pitch

The vertical screen length of the world $z$ axis is $\text{SCALE}\cos\varphi$ and the scale of the ground axis that points into the screen is $\text{SCALE}\sin\varphi$; the across axis stays at SCALE. At `LOWEST_PITCH` ($20.05^\circ$) heights are stretched to 1.151 and ground depth squeezed to 0.420; at $35.26^\circ$ they are 1.000 and 0.707; at $90^\circ$ (top-down, key `t`) heights vanish and the ground is seen at SCALE both ways, which is why the cube in the figure becomes a diamond.

## The inverse: from the screen back to the ground

Pointing and panning need the other direction, for points on the ground plane $z = 0$. `Camera::ground(dx, dy)` takes a screen offset in frame pixels from the view centre and returns the ground offset that appears there:

$$
\text{across} = \frac{dx}{\kappa}, \qquad \text{along} = \frac{dy}{\kappa\sin\varphi}, \qquad
\begin{pmatrix}\Delta x\\ \Delta y\end{pmatrix} = \begin{pmatrix} c & s\\ -s & c\end{pmatrix}\begin{pmatrix}\text{across}\\ \text{along}\end{pmatrix}.
$$

(The rotation here is the transpose of the one in `view`, since it is its inverse.) It divides by $\sin\varphi$, which is why `LOWEST_PITCH` is positive. A ground area $A$ appears with area $\kappa^2\sin\varphi\,A$, so top-down is where the ground is least compressed.

* **Panning** `pan(dx, dy)` adds `ground(dx, dy)` to the centre, so a drag of $n$ pixels moves the content by $n$ pixels whatever the pitch and rotation. Keys pan by `0.04 * width` pixels, wheel scrolls by `0.04 * height`.
* **Zooming at a point** `scale_at(f, dx, dy)` keeps the ground point under the screen offset fixed. That point is $P = c + G/\text{zoom}$ with $G$ the zoom-free part of `ground`; after multiplying the zoom by $f$ it would move to $c + G/(f\,\text{zoom})$, so the centre is shifted by $\frac{G}{\text{zoom}}(1 - 1/f)$, which is exactly `before - after` in the code. Ctrl and the wheel use $f = 1.1^{-(x+y)}$ about the pointer, measured from the point `(W/2, ORIGIN_Y H)` where the centre projects. `+`/`-` use 1.15 about the centre.
* **`look_at(p)`** centres on a *raised* point rather than the ground below it. The point appears above its ground point by `z / tan(pitch)` along the view direction, so the centre is moved to $p_{xy} - \frac{z}{\tan\varphi}(s, c)$. Setting the vertical image coordinate to zero: $\text{along}\,\sin\varphi = z\cos\varphi$ and $\text{across} = 0$.
* **`front(p, d)`** is the ground point $d$ world units from $p$ towards the viewer, $p + d\,(s, c)$: the lowest point on screen of a disc around $p$, used to place labels under a system.

## The code: easing, fitting, tour

### Goal and camera

`App` holds two cameras, `goal` and `camera`. Keys, `Home`, `r`, search, focus, the tour and the fit change the goal; direct mouse gestures (drag, wheel) change both at once and therefore apply immediately. Each frame the loop runs

```text
camera.approach(&goal, smoothing(elapsed, 0.2))
```

with $a = 1 - e^{-\Delta t/0.2}$ and `approach` moving each parameter a fraction $a$ of the way: the centre and the pitch linearly, the rotation along the shortest arc ($((\text{goal} - \text{rotation} + \pi) \bmod 2\pi) - \pi$), and the zoom *geometrically*, $\text{zoom} \leftarrow \text{zoom}\,(\text{goal}/\text{zoom})^a$, so that a zoom feels like a constant rate in log scale. Because $a$ is an exponential step, the ease is independent of frame rate: two steps of $\Delta t_1$ and $\Delta t_2$ leave the fraction $(1-a_1)(1-a_2) = e^{-(\Delta t_1+\Delta t_2)/0.2}$ of the gap, the same as one step over the sum. The time constant is 0.2 s.

| Input | Effect on the goal (or both cameras) |
|---|---|
| arrows, `w a s d` | `pan` by `0.04 * width` px |
| `+` / `=`, `-` | zoom by 1.15, 1/1.15 |
| `e`, `Q` | rotation $\pm\pi/2$ |
| PgUp, PgDn | `tilt(+0.12)`, `tilt(-0.12)`, clamped to `LOWEST_PITCH ..= pi/2` |
| `t` | pitch to `pi/2` if currently at most 1.2 rad, else back to `ISOMETRIC` |
| `Home` | refit (`fit = true`) |
| `r` | goal reset to `Camera::default()` (plus the row's `pitch`), refit |
| left drag (immediate) | `pan(-dx, -dy)` |
| other drag (immediate) | rotation $+0.01\,dx$, tilt $+0.008\,dy$ |
| Ctrl + wheel (immediate) | zoom about the pointer, factor $1.1^{-(x+y)}$ |
| Alt + wheel (immediate) | rotation $+0.08\,(x+y)$ |
| wheel | pan by `0.04 * height` px; on the Globe, turn and tilt the earth instead |

Zoom is clamped to $[0.3, 65]$ everywhere (`Camera::scale`).

### Fitting bounds

`Scene::fit(camera, W, H)` frames what the last frame drew. It projects the ground positions (or, if the view gave `stage.bounds`, those world points) with a probe camera at zoom 1 and centre 0, finds the extent $\Delta_u \times \Delta_v$ in view units and picks the zoom. The centre is `probe.ground(mid_u, mid_v)`, the ground point whose projection is the middle of the box.

$$
\text{zoom} = \begin{cases}
0.8\,\min\!\Bigl(\dfrac{W}{\Delta_u + 12},\; \dfrac{H}{\Delta_v + h}\Bigr), \quad h = \max(24,\; \text{tallest}\cos\varphi\,\text{SCALE} + 8) & \text{positions (City leaves headroom for the tallest building)}\\[2ex]
0.9\,\min\!\Bigl(\dfrac{W}{\Delta_u},\; \dfrac{H}{\Delta_v}\Bigr) & \text{explicit bounds}
\end{cases}
$$

For explicit bounds the centre is also lifted by $(0.57 - 0.5)H/\text{zoom}$ so that the box lands in the middle of the frame rather than at the 0.57 line. Views whose content moves by itself return a fixed `stage.bounds` envelope so the fit does not chase the simulation. While the camera "still frames the whole view" (`App::framed`) and no tour runs, a sample refits views whose bounds grow with data. The first frame snaps the camera to the fitted goal instead of easing there (`App::snap`).

### The tour

The idle tour starts after `--tour` seconds without input (default 20; 0 disables; any key or mouse event cancels it), or at once on `g`. It visits `targets[index % len]` for `TOUR_STEP = 6 s` each, or until a target no longer has a place. Targets interleave, for $i = 0..6$, the view's own heaviest targets (`Stage::tour`, or Orbit's system stars), the busiest process by CPU and the largest by memory, without repeats, so at most 18 targets. A visit clears the selection and sets the goal zoom to `fit_zoom * 2.2`; every frame the goal centre is set to the target's ground position, and in a view whose row declares `face` (only the Globe does) the camera is turned to put the target on the near side and `look_at` centres the point itself. Elsewhere the tour only pans and zooms. A row with `tour: false` ends any tour on entry.

## Into the GPU

`gpu.rs` and `shaders.wgsl` take frame pixels, not world points, so the camera never reaches the GPU. The vertex stage maps pixel $(X, Y)$ to clip space by

$$
x_{\text{ndc}} = \frac{2X}{W} - 1, \qquad y_{\text{ndc}} = 1 - \frac{2Y}{H}, \qquad w = 1,
$$

and the depth by the affine normalisation described in [Theory GPU and Parity](Theory-GPU-and-Parity). There is nothing to divide.

## Reading the picture

* **Size means nothing about distance.** Two equal bodies at different depths draw at the same size; only occlusion and the vertical position on screen say which is nearer. A body that rises in the frame is further along the view direction on the ground, or higher.
* **Height and depth are entangled at this pitch.** On screen a point at height $z$ and a point displaced along the view direction by $z/\tan\varphi$ coincide. This is the ambiguity every isometric picture has; the shadows City draws and the top-down toggle (`t`) are the ways out.
* **Rotation is by quarter turns for a reason.** `e` and `Q` turn by $\pi/2$, which keeps ground grids axis-aligned with the screen diagonal; free rotation is by Alt-wheel or a right drag.

## Where in the code

| What | Where |
|---|---|
| `Camera`, `view`, `ground`, `pan`, `scale`, `scale_at`, `tilt`, `viewer`, `look_at`, `front`, `approach`, `hold`, `release`, `SCALE`, `ISOMETRIC`, `LOWEST_PITCH`, `ORIGIN_Y` | [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs) |
| `Frame::project`, `Frame::locate`, `Scene::fit` | [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs) |
| Goal and camera, key and mouse handling, the tour, `TOUR_STEP` | [`src/main.rs`](https://github.com/tfpickard/isotop/blob/main/src/main.rs): `App::command`, `pointer_event`, `update_tour`, `visit`, `face`, `run` |
| `smoothing` | [`src/simulation.rs`](https://github.com/tfpickard/isotop/blob/main/src/simulation.rs) |
| Clip-space mapping | [`src/shaders.wgsl`](https://github.com/tfpickard/isotop/blob/main/src/shaders.wgsl): `clip`, `normalized` |

Figure: [`docs/figures/theory-isometric-axes.py`](https://github.com/tfpickard/isotop/blob/main/docs/figures/theory-isometric-axes.py).

## References

* J. D. Foley, A. van Dam, S. K. Feiner and J. F. Hughes, *Computer Graphics: Principles and Practice*, 2nd ed., Addison-Wesley, 1990 (parallel projections, orthographic and axonometric views).
* T. Akenine-Moller, E. Haines and N. Hoffman, *Real-Time Rendering*, 4th ed., CRC Press, 2018 (orthographic projection, and the depth buffer under it).
