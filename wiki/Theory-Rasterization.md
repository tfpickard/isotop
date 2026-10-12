# Theory Rasterization

The CPU rasterizer is `Frame::rasterize`: a few hundred lines of straightforward scan-conversion that turn a display list into three images, colour, depth and a pick buffer, one pixel at a time, on one thread. It is the reference implementation: the golden hashes of the test suite are taken from it, and the GPU path is built to reproduce it. This page derives the edge-function arithmetic it uses, states exactly which sampling convention each primitive follows (they are not all the same), accounts for what a frame costs and why, and says what the rasterizer deliberately does not do: anti-aliasing, perspective correction, early depth rejection and parallelism. The last of these is the subject of issue [#85](https://github.com/tfpickard/isotop/issues/85) and is context here, not code.

![Pixels scanned for a thin line drawn as one quad and as 8 px pieces](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/theory-hairline-cost.svg)

## Theory: scan conversion with edge functions

A triangle with screen vertices $a, b, c$ covers a pixel when the pixel's sample point $p$ lies inside it. Pineda (1988) observed that this is a test of three signs of *edge functions*. For an edge from $a$ to $b$ define

$$
E(a, b, p) = (p_x - a_x)(b_y - a_y) - (p_y - a_y)(b_x - a_x),
$$

which is affine in $p$, zero on the line through $a$ and $b$, and positive on one side of it. (It is the negative of the 2D cross product $(b-a)\times(p-a)$; only ratios of edge functions are ever used, so the overall sign cancels.) $E(a,b,c)$ is twice the signed area of the triangle, with a sign that depends on its winding. Let $A = E(a, b, c)$. The barycentric coordinates of $p$ are ratios of sub-areas:

$$
\lambda_a = \frac{E(b, c, p)}{A}, \qquad \lambda_b = \frac{E(c, a, p)}{A}, \qquad \lambda_c = \frac{E(a, b, p)}{A} = 1 - \lambda_a - \lambda_b .
$$

$p$ is inside (or on the boundary) iff all three are $\ge 0$. Dividing by the signed $A$ makes the test independent of winding: if $A < 0$ every $E$ inside the triangle is negative too, and the ratios are positive. Any attribute $f$ given at the vertices is interpolated as $f(p) = \lambda_a f_a + \lambda_b f_b + \lambda_c f_c$.

Two properties matter for cost. First, $E$ is affine, so stepping one pixel right changes it by the constant $b_y - a_y$ and one pixel down by $-(b_x - a_x)$; renderers exploit this with incremental evaluation. Second, nothing in the test needs the triangle's interior: only a bounding box to bound the scan.

**Perspective correctness.** Interpolating an attribute linearly in screen space is wrong for a perspective projection, where the correct interpolant is linear in $1/w$ (divide each attribute by $w$, interpolate, divide by interpolated $1/w$). Here $w$ is constant ([Theory Camera and Projection](Theory-Camera-and-Projection) shows the map is affine), so screen-space barycentric coordinates *are* the world-space ones: an affine map preserves ratios of areas. The plain formula above is exact, no correction exists to be omitted, and depth interpolated this way is exactly linear in the world coordinates.

**Fill rule.** Pixels whose sample point lies exactly on an edge shared by two triangles can be claimed by both, by neither, or by exactly one, depending on the convention. Hardware uses a tie-break such as top-left so that every such pixel is drawn once. isotop's CPU rasterizer uses none: the test is `>= 0` on all three coordinates.

## The code

### Frame::rasterize

In order:

1. Size the buffers: `pixels` to $3WH$ bytes, `depth` to $WH$ `f32` set to $-\infty$, `picks` to $WH$ `u32` set to `NONE`. Together 11 bytes per pixel, 22.8 MB at 1920 by 1080 (6.2 MB colour, 8.3 MB depth, 8.3 MB picks). The allocations are recycled from the previous frame (`Buffers`).
2. `paint_sky`: every pixel gets the backdrop colour plus haze (see [Theory Scene and Display List](Theory-Scene-and-Display-List)). Per column $p(x/W, 0)$ is computed once, per row $p(y/H, 1.9)$ once; when the haze is non-zero (`Sky::hazy`: any haze channel above 0.5) each pixel costs a multiply-add and a clamp per channel, and otherwise a row is a plain fill.
3. Take the item list out of the frame and walk it in order, dispatching each `Item` to `triangle`, `segment`, `ball`, `halo`, `ray`, `pixel` (stars), `glyph` or `splat`, then put the list back.

There is no tiling, no binning, no sorting and no culling beyond clamping loops to the frame. Painter's order plus a depth test decides everything.

### Sampling conventions

The primitives do not share one convention, and the difference is visible at the half-pixel scale:

| Primitive | Sample point of pixel $(x, y)$ | Coverage rule |
|---|---|---|
| `Triangle` | $(x + \tfrac12,\; y + \tfrac12)$ | $\lambda_a, \lambda_b, \lambda_c \ge 0$ |
| `Sphere` (`ball`), `Pick` (`splat`) | $(x, y)$, the integer point | $\lvert n\rvert^2 \le 1$ |
| `Glow` (`halo`) | $(x + \tfrac12,\; y + \tfrac12)$ | distance $< R$ |
| `Line` (`segment`), `Beam` (`ray`), `Star` | pixel $= \text{trunc}(\text{point})$, so no sampling at all | one pixel per major-axis step |
| `Glyph` | origin truncated to an integer | each set font bit fills a `scale` by `scale` block |

Sampling a sphere at the integer point means that a body centred at $(c_x, c_y)$ with integer coordinates is symmetric about the *centre of pixel* $(c_x, c_y)$, while a triangle vertex at the same coordinates is the *corner* of that pixel; a sphere drawn at a triangle's vertex is half a pixel up and to the left of where a pixel-centre convention would put it. The GPU shader samples the same way (`floor(position.xy)` for balls and splats, the raw pixel centre for glows) so that the two rasterizers agree. A consequence of integer-lattice sampling is that the number of pixels a sphere covers is the Gauss circle count $N(r)$ (the number of integer points with $i^2 + j^2 \le r^2$): 37 for the minimum body radius 3.5, 81 for $r = 5$ (a unit test asserts it for the pick splat), 317 for $r = 10$, 1257 for $r = 20$, against $\pi r^2$ = 38.5, 78.5, 314.2, 1256.6.

### triangle

```text
area = E(a, b, c);  if |area| < 0.001 { return }          // degenerate
bbox = [floor(min x), ceil(max x)] x [floor(min y), ceil(max y)], clamped to the frame
for y in bbox.y, x in bbox.x:
    w0 = E(b, c, (x+.5, y+.5)) / area
    w1 = E(c, a, (x+.5, y+.5)) / area
    w2 = 1 - w0 - w1
    if w0 >= 0 && w1 >= 0 && w2 >= 0:
        pixel(x, y, w0*a.z + w1*b.z + w2*c.z, color, pick)
```

Details worth knowing: both windings draw (no back-face culling); the third coordinate is derived as $1 - w_0 - w_1$ rather than computed from $E(a,b,p)$, so a sample exactly on edge $ab$ is decided by rounding; a pixel on a shared edge is drawn by both triangles and the later one wins (equal colour on a quad's diagonal, so harmless; different colours on adjacent faces of a building, so a deterministic one-pixel seam owned by the later face). The area threshold is on twice the area in square pixels. Per pixel in the box the cost is two edge evaluations and two divisions, whether or not the pixel is covered; issue #85 lists the two divisions per pixel as a target.

### pixel: the depth test

```text
if depth < depth_buffer[i] { return }       // pass when depth >= stored
depth_buffer[i] = depth;  picks[i] = pick;  pixels[i] = color
```

Larger depth is nearer. Ties pass, so the later item wins them. There is no early rejection: a triangle behind a wall still pays for the edge functions of every pixel in its box before the depth test discards the covered ones.

### ball, splat, halo, segment, ray

* **`ball`** scans the box $[c \mp \text{outer}]$ with outer $= r + 3$ if selected else $r + 1$. Per pixel: two divisions ($n_x$, $n_y$ by $r$), $d = n_x^2 + n_y^2$; inside, one square root, the light, `tint`, and `pixel(..., c_z + n_z R, ...)`. The derivation is in [Theory Scene and Display List](Theory-Scene-and-Display-List). Cost: $(2\,\text{outer} + 1)^2$ pixel visits.
* **`splat`** (an `Item::Pick`) is `ball` without colour or depth writes: where $d \le 1$ and the disc's depth is at least the stored depth, only `picks[i]` is written.
* **`halo`** scans $[\lfloor p - R\rfloor, \lceil p + R\rceil]^2$, not clamped to the frame (the blend bounds-checks), computing a `hypot` per pixel and blending where $d < 1$. Cost $\approx (2R+2)^2$ visits for $\pi R^2$ blends, so the square of the radius.
* **`segment`** (a `Line`): `steps = ceil(max(|dx|, |dy|))`; for $i = 0..=\text{steps}$ it takes $t = i/\max(\text{steps}, 1)$ and a pixel at `trunc(a + (b - a) t)`, depth interpolated plus `LINE_DEPTH_BIAS = 0.02`, pick `NONE`. A digital differential analyser: one pixel per step of the major axis, both endpoints included, no width, no sub-pixel placement.
* **`ray`** (a `Beam`) is the same walk, capped at 8000 steps, blending instead of overwriting and touching neither depth nor picks. `stroke` and `circle`, the tour's annotation, are the same with three pixels per step and a 4000-step cap, applied after rasterization.

### Complexity

Write $W \times H$ for the frame. A frame's CPU cost is, up to constants,

$$
T \;\approx\; \underbrace{c_0\,WH}_{\text{clear and sky}} \;+\; \sum_{\text{tri}} c_t\,B_i \;+\; \sum_{\text{ball}} c_b\,(2r_j + 2)^2 \;+\; \sum_{\text{glow}} c_g\,(2R_k + 2)^2 \;+\; \sum_{\text{line, beam}} c_\ell\,(\ell_m + 1),
$$

where $B_i$ is a triangle's *bounding-box* area, not its area. Three corollaries follow, and the repository's conventions are shaped by them:

* **Fill dominates**, since each term scales with screen area at fixed world content: halving the width quarters the first term and, for bodies sized in world units, quarters the sphere and glow terms too. At 480 by 270 the same scenes take 4 to 12 ms where 1080p takes 25 to 80 (issue #85).
* **Thin diagonal geometry is quadratic in length** unless it is cut: a line drawn as a quad has a bounding box of area $\sim \ell^2/2$ at 45 degrees against a covered area of $\ell w$. `Frame::hairline` cuts it into pieces of at most 8 px, which makes the cost linear (figure: at 45 degrees a 1000 px stroke scans about 1.0 million pixels as one quad and about 16 thousand in pieces, 62 times fewer, for 1100 covered). `docs/VIEW_CONVENTIONS.md` records the rule ("a triangle scans its whole bounding box, so split long thin geometry") along with the glow rule and the fact that the sky is painted per pixel.
* **Overdraw is paid in full.** There is no hierarchical depth, no front-to-back ordering and no early-out, so a dense view costs the sum of its parts' pixel footprints, visible or not.

**Measured here.** One run on this wiki's build machine, a shared 4-vCPU Xeon VM at about 2.8 GHz under unrelated load, so treat the numbers as indicative: `--demo --renderer cpu --benchmark 20`, 192 processes, minimum of six runs, milliseconds per frame including recording and rasterization, excluding terminal transport.

| View | 480 x 270 | 1920 x 1080 | Ratio (pixels: 16) | Slope between the two, ms per Mpx |
|---|---|---|---|---|
| city | 3.4 | 42.3 | 12.4 | 20.0 |
| orbit | 3.0 | 22.0 | 7.4 | 9.8 |
| ripple | 11.3 | 57.3 | 5.1 | 23.7 |
| flow | 9.2 | 29.3 | 3.2 | 10.3 |
| cores | 4.4 | 39.3 | 8.9 | 17.9 |

Every ratio is below 16: each view has a fixed part (recording and simulating, which does not depend on resolution) and a part proportional to pixels, and the second is 10 to 24 ms per megapixel, 10 to 24 ns per pixel of the frame. The intermediate widths I measured (720, 960 and 1440) were not monotone enough on this machine to quote. Issue #85 reports 50.9, 25.2, 58.8, 44.4 ms for City, Orbit, Ripple and Flow at 1080p on another 4-core machine, the same order. The performance budget in `docs/VIEW_CONVENTIONS.md` is 10 fps at 1080p with 192 processes and 5 fps at 1000 processes.

### Parallelism: not there yet

`Frame::rasterize` is single-threaded: `render.rs` contains no thread, scope or channel. Issue #85 (A26, open) proposes row-banded parallelism with `std::thread::scope`: split the frame into horizontal bands, one per core, each band rasterizing the *whole* item list clipped to its rows. The argument that the output would be bit-identical is short and worth stating: a pixel's final value is a function of the sequence of items that cover it, taken in list order, and each band owns a disjoint set of pixels and walks the list in the same order, so every pixel sees the same sequence as before. Blends read only the same pixel, depth and picks are per pixel, so nothing crosses a band boundary. The same issue proposes evaluating edge functions in fixed-point integers (exact, and a clean route to a top-left fill rule), hoisting $1/A$, and replacing `hypot` only where bit-identical. Exactness is the constraint, because the golden test hashes colour, pick *and depth* buffers.

## Reading the picture

* **No anti-aliasing.** Every edge is a staircase. Sphere silhouettes are lattice discs (counts above), triangle edges are pixel-centre decisions, lines are one pixel per major-axis step. Issue #83 lists anti-aliasing as out of scope. How a terminal rescales a 1920-wide image into its cells is the terminal's business ([Theory Terminal Graphics](Theory-Terminal-Graphics)).
* **Depth is exact within a primitive and approximate between them.** Triangle depth is the exact linear interpolant; sphere depth is exact for a sphere of world radius $R$; a line's depth gets a 0.02-unit bias so that it wins against the surface it lies on. A fragment that loses a depth tie by less than a rounding error is a one-pixel flicker, not a bug.
* **What the picture hides:** the order of translucent items (list order decides), and the half-pixel offsets between primitive families.

## Where in the code

| What | Where |
|---|---|
| `Frame::rasterize`, `paint_sky`, `pixel`, `triangle`, `segment`, `ball`, `splat`, `halo`, `ray`, `glyph`, `stroke`, `circle`, `hairline` | [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs) |
| Constants used by the shaded sphere, ring and line bias | [`src/render/shared.rs`](https://github.com/tfpickard/isotop/blob/main/src/render/shared.rs) |
| Golden hashes of pixels, picks and depth | [`tests/golden.txt`](https://github.com/tfpickard/isotop/blob/main/tests/golden.txt), `every_view_matches_its_golden_hash` in [`src/main.rs`](https://github.com/tfpickard/isotop/blob/main/src/main.rs) |
| Benchmark | `isotop --demo --renderer cpu --benchmark N` ([`src/main.rs`](https://github.com/tfpickard/isotop/blob/main/src/main.rs): `run`) |

Figure: [`docs/figures/theory-hairline-cost.py`](https://github.com/tfpickard/isotop/blob/main/docs/figures/theory-hairline-cost.py) counts bounding-box pixels exactly as the CPU loop would, for a fixed sub-pixel start.

## References

* J. Pineda, "A Parallel Algorithm for Polygon Rasterization", *Computer Graphics* 22(4) (SIGGRAPH 1988), 17-20.
* T. Akenine-Moller, E. Haines and N. Hoffman, *Real-Time Rendering*, 4th ed., CRC Press, 2018 (rasterization, fill rules, perspective-correct interpolation).
* J. D. Foley, A. van Dam, S. K. Feiner and J. F. Hughes, *Computer Graphics: Principles and Practice*, 2nd ed., Addison-Wesley, 1990 (scan conversion, the digital differential analyser).
* isotop issues [#83](https://github.com/tfpickard/isotop/issues/83) (known CPU/GPU divergences) and [#85](https://github.com/tfpickard/isotop/issues/85) (CPU throughput).
