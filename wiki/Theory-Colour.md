# Theory: Colour

Every colour that means something about a process or a CPU core lives in one 174-line file,
[`src/palette.rs`](https://github.com/tfpickard/isotop/blob/main/src/palette.rs): four kind colours, three state
colours, three core colours, eight district colours, four stone colours, and two arithmetic helpers, `mix` and
`tint`. This page asks what those numbers are, what space the arithmetic happens in, what it costs there, and
how well a viewer can tell the colours apart, including a viewer with colour-vision deficiency. The numbers
are computed from the actual palette values. It ends with the two planned changes: a colour-blind-safe
palette ([#104](https://github.com/tfpickard/isotop/issues/104)) and a blackbody colour table
([#32](https://github.com/tfpickard/isotop/issues/32)).

![The isotop palette, and the same swatches as a protanope, a deuteranope and a tritanope would see them; below, mix and tint compared with linear-light blending](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/colour-palette.svg)

*Rows: normal vision; protanopia, deuteranopia, tritanopia (Machado et al. 2009, severity 1.0, section 5).
Below: a `mix` of teal and amber in the encoded space the code uses, against the same blend in linear light,
with the relative luminance of each step. Colour-vision simulations are indicative.*

## 1. Theory: what a number like `[72, 183, 199]` is

This section is standard colour science, independent of isotop.

### 1.1 Encoded values and linear light

A display byte is not proportional to light. The sRGB standard (IEC 61966-2-1) defines the **transfer function**
that maps an encoded value $c\in[0,1]$ to linear-light intensity:

$$
c_{\rm lin}=\begin{cases}c/12.92 & c\le0.04045\\ \bigl((c+0.055)/1.055\bigr)^{2.4} & \text{otherwise,}\end{cases}
\qquad
c=\begin{cases}12.92\,c_{\rm lin} & c_{\rm lin}\le0.0031308\\ 1.055\,c_{\rm lin}^{1/2.4}-0.055 & \text{otherwise.}\end{cases}
$$

Over most of its range it is close to a power law of exponent 2.2. Physics (adding two lights, blurring,
averaging, anti-aliasing, lighting) is linear in *light*, so it is correct on $c_{\rm lin}$; encoded bytes are
spaced to be roughly uniform in *perceived lightness*, which is why 8 bits suffice. The **relative luminance**
of a colour is

$$
Y=0.2126\,R_{\rm lin}+0.7152\,G_{\rm lin}+0.0722\,B_{\rm lin}\in[0,1],
$$

and the CIE lightness is $L^*=116\,f(Y)-16$ with $f(t)=t^{1/3}$ above $0.008856$. Contrast between two colours
(WCAG 2.x) is $(Y_{\rm hi}+0.05)/(Y_{\rm lo}+0.05)$; 3:1 is the WCAG threshold for graphical objects and 4.5:1 for
body text. A colour difference that a human sees as such is measured in a perceptual space: $\Delta E_{00}$
(CIEDE2000; Sharma, Wu and Dalal 2005) is the current standard, with a value near 1 for a just-noticeable
difference under controlled viewing conditions, and a good deal more needed for small isolated marks on a
busy ground.

### 1.2 Colour-vision deficiency

About one man in twelve has a reduced or absent response of one of the three cone classes: L (protan), M
(deutan) or S (tritan). A dichromat's colours collapse onto a surface in colour space; the simulation of
Machado, Oliveira and Fernandes (2009) models that as a $3\times3$ matrix on **linear** RGB for severity 0 to 1.
Pairs that differ mainly along the lost axis (red against green for protan and deutan, blue against yellow for
tritan) become hard to tell apart. A palette is "safe" when the minimum $\Delta E_{00}$ over its pairs stays above
a threshold *after* simulation. Section 5 does this for the shipped colours.

## 2. The code: the palette

All are `render::Color = [u8; 3]`, sRGB-encoded bytes.

| constant | RGB | hex | $Y$ | $L^*$ | contrast on Dusk top / bottom | on Space | on Sea top | meaning |
|---|---|---|---|---|---|---|---|---|
| `KERNEL` | 104, 122, 156 | `#687a9c` | 0.193 | 51.0 | 4.49 / 4.25 | 4.62 | 2.27 | kernel threads: slate |
| `SYSTEM` | 72, 183, 199 | `#48b7c7` | 0.394 | 69.0 | 8.20 / 7.78 | 8.45 | 4.16 | system services: teal |
| `SESSION` | 236, 178, 92 | `#ecb25c` | 0.504 | 76.3 | 10.25 / 9.72 | 10.56 | 5.19 | the user's session: amber |
| `CONTAINER` | 178, 136, 232 | `#b288e8` | 0.329 | 64.1 | 7.01 / 6.64 | 7.22 | 3.55 | containers: violet |
| `STATE_ZOMBIE` | 213, 97, 126 | `#d5617e` | 0.242 | 56.3 | 5.40 / 5.12 | 5.56 | 2.74 | state Z |
| `STATE_STOPPED` | 225, 161, 75 | `#e1a14b` | 0.420 | 70.9 | 8.69 / 8.24 | 8.95 | 4.40 | state T or t |
| `STATE_BLOCKED` | 222, 94, 73 | `#de5e49` | 0.240 | 56.1 | 5.36 / 5.09 | 5.53 | 2.72 | state D |
| `UNKNOWN_CORE` | 140, 160, 196 | `#8ca0c4` | 0.347 | 65.5 | 7.34 / 6.96 | 7.56 | 3.72 | a core whose kind is unknown |

`PERFORMANCE = SESSION` and `EFFICIENCY = SYSTEM`: performance cores are the session's amber, efficiency
cores the system's teal. `DISTRICTS` is eight colours, cycled by `group % 8` for the City: teal
(72, 183, 199), blue (122, 145, 229), orchid (190, 135, 215), tan (211, 159, 91), sea green (98, 185, 149), rose
(212, 116, 140), pale blue (149, 171, 213), olive (166, 185, 104). `stone_color(kind)` gives the Necropolis
stones, deliberately duller: slate (104, 112, 132), green granite (112, 138, 134), sandstone (166, 142, 108),
lilac marble (134, 118, 160).

The backdrops (`render::shared`, in 0 to 255 units): Space is flat (5, 8, 18); Dusk runs from (8, 13, 25) at the
top to (14, 20, 33) at the bottom; Sea from (24, 72, 96) at the top to (6, 22, 38); Void is black. Their
luminances are 0.0025, 0.0041 to 0.0070, 0.057 to 0.0075 and 0. All kind colours are well above 4:1 on Dusk and
Space; **slate (`KERNEL`) is the weakest, 4.25:1 on the lighter bottom of Dusk, and 2.27:1 on the bright top of the
Sea backdrop**, below the 3:1 graphical-object threshold. The kind colours are jittered darker (section 3.2),
which lowers the contrast further: the darkest jittered slate, (89, 104, 134), has $Y=0.137$ and a contrast of 3.3:1
on the bottom of Dusk.

Functions:

* `state_color(state)`: `Z` zombie, `T` or `t` stopped, `D` blocked, else none. A state colour **overrides** the
  kind colour.
* `process_color(process, group)` (City): the state colour, or `DISTRICTS[group % 8]`.
* `kind_color(process)` (Orbit, Ripple, Flow, Cores, Cells, Strata, Globe, Coop, Ptolemy, Tesseract): the state
  colour, else the kind base brightness-scaled per pid (section 3.2). State colours are **not** jittered.
* `core_color(kind)` and `core_name(kind)` ("performance", "efficiency", "unknown").
* Colours used by only one view stay in that view (Reef's own palette, the Globe's coastline), and the terminal
  overlay colours (`Tone::Quiet` 150, 172, 198; `Bright` 214, 226, 240; `Tag` 255, 226, 170 on 12, 19, 31; the popup
  border 92, 122, 158 on 12, 19, 31; the status text 180, 199, 218) are in `terminal.rs`, outside `palette.rs`.

## 3. The arithmetic: everything happens in encoded space

### 3.1 No transfer function anywhere

`Color` is an encoded byte triple, and **neither rasterizer applies or removes a transfer function**. The CPU
works on the bytes directly (`Frame::blend`, `Frame::ball`, `tint`). The GPU render target is
`wgpu::TextureFormat::Rgba8Unorm`, not the `...Srgb` variant that would encode on write and decode on read, and
its alpha blending is the plain `BlendState::ALPHA_BLENDING` on those values; the sphere shader reproduces the
CPU's integer arithmetic (`clamp(floor(color * 255 * light), 0, 255)`). So every operation below is an
operation on *encoded* values. That is a design choice with consequences, which this section quantifies. (The
CPU/GPU agreement it preserves is described on [GPU and parity](Theory-GPU-and-Parity).)

### 3.2 `tint` and the per-pid jitter

`tint(color, scale)` is `(v as f32 * scale).clamp(0.0, 255.0) as u8` per channel: a multiplication of the
encoded value, saturating at 255, truncating. Because an encoded value is approximately $Y^{1/2.2}$, scaling it
by $s$ scales luminance by about $s^{2.2}$. Measured on `SESSION` (relative luminance 0.5045):

| `scale` | result | $Y$ | $Y/Y_{\rm base}$ | pure $s^{2.2}$ |
|---|---|---|---|---|
| 0.50 | 118, 89, 46 | 0.112 | 0.222 | 0.218 |
| 0.72 | 169, 128, 66 | 0.243 | 0.481 | 0.485 |
| 0.86 | 202, 153, 79 | 0.359 | 0.712 | 0.718 |
| 1.00 | 236, 178, 92 | 0.504 | 1.000 | 1.000 |
| 1.10 | 255, 195, 101 | 0.612 | 1.214 | 1.233 |
| 1.20 | 255, 213, 110 | 0.700 | 1.387 | 1.493 |

For $s\le1$ the exponent law holds to a percent; above 1 the red channel clips at 255 first, the colour loses
saturation toward yellow, and the brightening falls short of $s^{2.2}$. The sphere light of the rasterizer,
$\text{light}=\min(0.25+\max(0,\,\mathbf n\cdot\mathbf l),\,1.2)$ (constants `SPHERE_AMBIENT`, `SPHERE_LIGHT_*`),
is a `tint` with a scale in $[0.25,1.2]$: a shaded side at 0.25 has about $0.25^{2.2}=5\%$ of the luminance of
the lit side. That is a contrast ratio of 20 to 1 from a mere factor-of-four in the number, and it is why
spheres look deeply shaded rather than lightly.

`kind_color` scales its base by `0.86 + 0.24 * j`, where `j = (hash::knuth(pid) >> 24) / 255` takes 256 values in
$[0,1]$, so the scale spans $[0.86,1.10]$ "so neighbours differ". The cost, measured:

| kind | darkest | brightest | $\Delta E_{00}$ between them | luminance ratio |
|---|---|---|---|---|
| `KERNEL` | 89, 104, 134 | 114, 134, 171 | 12.0 | 1.71 |
| `SYSTEM` | 61, 157, 171 | 79, 201, 218 | 12.4 | 1.73 |
| `SESSION` | 202, 153, 79 | 255, 195, 101 | 12.1 | 1.71 |
| `CONTAINER` | 153, 116, 199 | 195, 149, 255 | 12.5 | 1.72 |

A jitter of about 12 $\Delta E_{00}$ and a luminance ratio of 1.7 *within* a kind is of the same size as the
differences *between* kinds in the worst case (section 5: 8.7 for teal against violet for a deuteranope). The kinds
are separated by **hue**, and lightness is spent on the pid; a viewer cannot read kind from brightness.

The jitter hashes the pid alone (`// pid only: kept for pixel stability, see A33`), so a reused pid gets its
predecessor's shade; see [Determinism](Theory-Determinism#33-the-legacy-hashes-and-why-they-stay).

### 3.3 `mix`: a straight line between two encoded colours

`mix(a, b, t)` is per channel `(a + (b - a) * t) as u8`: linear interpolation of the **encoded** bytes,
truncated. It is exact at the ends (`t = 0` gives `a`, `t = 1` gives `b`, because the arithmetic is exact
on small integers) and extrapolates, then saturates, outside $[0,1]$. Two consequences.

**Midpoints are darker than a physical mixture.** Averaging light is averaging $c_{\rm lin}$; averaging bytes
averages $\approx Y^{1/2.2}$, whose midpoint is below the encoded value of the averaged light. For the two
extreme cases and two palette pairs:

| pair, $t=\tfrac12$ | `mix` result | $Y$ of `mix` | linear-light blend | $Y$ of blend | ratio |
|---|---|---|---|---|---|
| white, black | 127, 127, 127 | 0.212 | 188, 188, 188 | 0.503 | 0.42 |
| red, green | 127, 127, 0 | 0.197 | 188, 188, 0 | 0.467 | 0.42 |
| `SESSION`, `CONTAINER` | 207, 157, 162 | 0.400 | 210, 159, 180 | 0.418 | 0.96 |
| `SYSTEM`, `KERNEL` | 88, 152, 177 | 0.277 | 90, 156, 179 | 0.292 | 0.95 |
| `SYSTEM`, `SESSION` ($t=\tfrac12$) | 154, 180, 145 | 0.416 | 179, 181, 157 | 0.451 | 0.92 |

The palette's colours were chosen close in lightness, so `mix` loses only 4 to 8 percent of the blended
luminance; between very different lightnesses it loses more than half (white against black, the figure's
bottom panel). Gradients, fades and glow falloffs between a bright and a dark colour dip in the middle, which
reads as a faint outline around bright objects on dark grounds. It is also **what the GPU draws too**, so the
two renderers agree.

**Truncation is biased.** `as u8` rounds toward zero, so the expected error is $-0.5$ of a level per channel
per operation; repeated blends (a glow over a beam over a sphere) accumulate a few levels of darkening. It is
smaller than the 6-level tolerance of the planned blackbody test and well below perception, and it is the
reason the golden hashes are exact: deterministic truncation, not rounding that depends on a libm.

### 3.4 Blending and haze

`Frame::blend(x, y, color, weight)` is `channel += (target - channel) * weight`, truncated: the same
encoded-space lerp, used by halos (`strength * (1 - d)^2` falloff), rays and the pressure haze. The sky haze
is *added* in 0 to 255 units (`min(base + haze * density, 255)`) before the `floor`. None of these are physically
additive in light; they are compositing operators on display values, consistent between the backends.

## 4. Perceptual notes

* **Hue carries kind, lightness carries identity noise, saturation is high.** The four kind colours have
  $L^*$ of 51, 64, 69, 76: slate is the darkest and amber the lightest, and the jitter moves each by $\pm0.1$ in
  scale.
* **Teal and amber are the only pair of unrelated hues placed at the same lightness class** ($L^*$ 69 against
  76), which is why they are also the two core colours: they read as "this machine's two kinds of core"
  without implying more or less.
* **Slate is a ground colour.** At 4.25:1 on Dusk bottom and 2.27:1 on the Sea backdrop it recedes, which suits
  kernel threads (many, uninteresting) and fails for any view that puts them on the lighter backdrops.
* **Warm colours dominate attention.** Amber (session) is the brightest and most saturated of the four, the
  user's own processes. That agrees with the mapping, and it is the same colour as the stopped state
  (section 5), which does not.
* **Simultaneous contrast and adaptation** are not modelled: colours are chosen against the dark backdrops
  only, and the selection ring (237, 220, 154) is contrasted with them only by being bright: against
  `SESSION` its contrast is 1.38:1 ($\Delta E_{00}$ 14.2), the weakest of the kinds; against `KERNEL` 3.15:1.

## 5. How well can the colours be told apart?

All pairs among the eight colours of the table (four kinds, three states, the unknown core), by $\Delta E_{00}$,
for normal vision and under the three deficiencies. CIEDE2000 was implemented from the formulae and checked
against seven of the published test pairs of Sharma et al. (2005), agreeing to the four printed decimals. The
colour-vision matrices (Machado et al. 2009, severity 1.0, in linear RGB; each row sums to 1 so that greys stay
grey, and I checked that) were transcribed by the author from memory of the published table and are marked
**(unverified)**; treat the numbers as indicative, and the issue's planned unit test as authoritative.

| vision | closest pairs among all eight ($\Delta E_{00}$) |
|---|---|
| normal | **`SESSION`-`STOPPED` 4.5**; `KERNEL`-`UNKNOWN_CORE` 13.1; `CONTAINER`-`UNKNOWN_CORE` 14.0; `ZOMBIE`-`BLOCKED` 16.3 |
| protanopia | `SESSION`-`STOPPED` 4.6; `SYSTEM`-`UNKNOWN_CORE` 5.8; `CONTAINER`-`UNKNOWN_CORE` 10.1; `KERNEL`-`ZOMBIE` 11.0 |
| deuteranopia | **`SYSTEM`-`UNKNOWN_CORE` 1.6**; `SESSION`-`STOPPED` 3.8; `CONTAINER`-`UNKNOWN_CORE` 8.0; `SYSTEM`-`CONTAINER` 8.7 |
| tritanopia | `SESSION`-`STOPPED` 4.4; `ZOMBIE`-`BLOCKED` 4.5; `KERNEL`-`UNKNOWN_CORE` 13.0 |

Reading the table:

* **The four kind colours alone are well separated**: the minimum over their six pairs is 18.2 (normal, slate
  against violet), 14.7 (protan), 8.7 (deutan, teal against violet) and 20.9 (tritan, slate against teal).
  For comparison the seven chromatic colours of the Okabe-Ito palette (Okabe and Ito 2008, built for exactly
  this purpose) have minima of 21.7, 12.3, 11.5 and 11.1. The shipped kinds are already competitive with a
  purpose-built palette, which is why the issue calls for *checking* rather than redesigning them.
* **`STATE_STOPPED` collides with `SESSION` for everyone**: (225, 161, 75) against (236, 178, 92), $\Delta E_{00}$
  4.5. Worse, `kind_color` jitters the session amber by 0.86 to 1.10, and the stopped colour lies **inside that
  range**: the nearest jittered session shade is (220, 165, 85), at $\Delta E_{00}$ 2.3 from the stopped colour,
  and for 63 percent of the 256 jitter values the difference is under 5, for 23 percent under 3. In Orbit, Ripple,
  Flow and every other view that uses `kind_color`, **a stopped process in the user's session is, by colour
  alone, indistinguishable from a running one**. The state is carried by other cues in some views (the
  inspector, motion), not by the colour that was meant to carry it.
* **`UNKNOWN_CORE` is a steel blue between slate and teal**, and a deuteranope sees it 1.6 away from the
  efficiency-core teal. It only matters if both appear in one view (a part-hybrid machine reporting some cores
  as unknown).
* **`ZOMBIE` and `BLOCKED`** (rose against red-orange) are 16.3 apart normally and 4.5 for a tritanope.
* **The eight district colours** are a categorical scale beyond what hue alone can carry: their closest pair
  is 8.6 apart normally, and 2.3 (protan), 2.4 (deutan), 3.7 (tritan) under simulation. Districts are
  spatially separate blocks of plots, which is what rescues the City.

## 6. Planned: a colour-blind-safe palette (issue #104)

> **Planned, not yet implemented.** Specified in [issue #104](https://github.com/tfpickard/isotop/issues/104)
> ("F10: Colour-blind safe palette"). There is no `--palette` option in `main.rs`.

The issue asks for `--palette default|safe`; the active palette is chosen once at startup so that no view takes
a palette argument. The safe palette is to be chosen "by measurement, not by eye": the four kind colours, the
core performance/efficiency colours and the three state colours must stay distinguishable under protanopia,
deuteranopia and tritanopia simulation (Machado et al. 2009 matrices) with a minimum pairwise $\Delta E_{00}$ above
a stated threshold, in a unit test (`safe_palette_is_distinguishable_under_common_colour_vision_deficiencies`)
so later edits cannot regress it. The default palette's pixels must not change (the golden hashes hold with
`--palette default`), and legends that name colours ("teal = session") are to be generated from `palette.rs`.
Section 5 shows what the test will find on the shipped palette: the stopped/session pair already fails any
threshold above about 4.5, and the unknown-core/system pair fails under deuteranopia. That is a concrete
reason the check should include the *state against kind* pairs, which the issue's list does.

## 7. Planned: the blackbody table (issue #32)

> **Planned, not yet implemented.** Specified in [issue #32](https://github.com/tfpickard/isotop/issues/32)
> ("B5: Add the blackbody colour table"), `src/blackbody.rs` does not exist. It will serve the planned Horizon
> and HR views.

A star's colour is its temperature. The plan is a pure module mapping $T\in[1000,40000]$ K to a display colour.

**Physics.** A blackbody at temperature $T$ emits the spectral radiance
$B_\lambda(T)\propto\lambda^{-5}\bigl(e^{c_2/(\lambda T)}-1\bigr)^{-1}$, $c_2=hc/k=1.4387769\times10^{-2}$ m K, with
its peak at $\lambda_{\max}=b/T$ ($b=2.8978\times10^{-3}$ m K, Wien's law): 2 898 nm at 1 000 K, 966 nm at 3 000 K,
502 nm at 5 778 K, 290 nm at 10 000 K. The eye integrates the spectrum against the colour-matching functions
$\bar x,\bar y,\bar z$: $X=\int B_\lambda\bar x\,d\lambda$, and so on.

**The recipe in the issue.** Multi-lobe Gaussian fit of the CIE 1931 2-degree observer by Wyman, Sloan and
Shirley (2013) (no tabulated CIE data, for licence reasons); integrate 360 to 830 nm in 1 nm steps in `f64`;
convert with the IEC 61966-2-1 matrix to linear sRGB; clip negative channels to 0; divide by the largest
channel; store 256 entries at $T_k=1000\cdot40^{k/255}$ (geometric, 1.46 percent per step, "because colour changes
fastest at low temperatures"); interpolate linearly in $\ln T$; and, for `color(kelvin, brightness)`, scale by the
brightness **then apply the sRGB transfer function and round**, because "frames hold sRGB-encoded bytes, so
linear values must never go straight into a `Color`". This is exactly the point of section 3.1: the table is in
linear light, the frame is in encoded bytes, and the module is the one place a transfer function is applied.

**A check of the recipe.** The prototype technique in `docs/mockups/code/hr.py` is the recipe in NumPy. I
re-implemented it in the standard library (the same lobe constants, 1 nm steps, same matrix) and computed:

| $T$ (K) | linear, max-normalised | encoded | hex | $Y$ |
|---|---|---|---|---|
| 1 000 | 1, 0.029, 0 | 255, 47, 0 | `#ff2f00` | 0.233 |
| 2 000 | 1, 0.266, 0.008 | 255, 141, 21 | `#ff8d15` | 0.404 |
| 3 000 | 1, 0.484, 0.155 | 255, 185, 110 | `#ffb96e` | 0.570 |
| 4 000 | 1, 0.659, 0.379 | 255, 212, 166 | `#ffd4a6` | 0.711 |
| 5 000 | 1, 0.795, 0.630 | 255, 231, 208 | `#ffe7d0` | 0.827 |
| 6 500 | 1, 0.945, 0.993 | 255, 249, 254 | `#fff9fe` | 0.960 |
| 10 000 | 0.610, 0.695, 1 | 205, 217, 255 | `#cdd9ff` | 0.699 |
| 20 000 | 0.407, 0.536, 1 | 171, 193, 255 | `#abc1ff` | 0.542 |
| 40 000 | 0.344, 0.480, 1 | 158, 184, 255 | `#9eb8ff` | 0.489 |

Validation against an independent source: the CIE chromaticity $(x,y)$ of the same spectra against the cubic
approximation of the Planckian locus by Kim et al. (2002) (coefficients remembered by the author,
**unverified**, as are the matrices in section 5) agrees to $1\times10^{-4}$ at 6 500 K and 10 000 K, $3\times10^{-4}$
at 5 000 K, $6\times10^{-4}$ at 4 000 K, $1\times10^{-3}$ at 2 856 K and $3\times10^{-3}$ at 1 900 K, the largest
discrepancy being at the cool end where the Wyman fit and the polynomial are both least accurate. The 6 500 K
point is $(0.3135,0.3237)$, which is the well-known chromaticity of a 6 500 K blackbody. So the lobe constants
reproduce a physically reasonable locus.

What the table shows about the design:

* **Below about 1 910 K the blue channel is negative before clipping**: the blackbody colour is outside the sRGB
  gamut, and clipping to zero (as the recipe specifies) is a hue shift. At 1 000 K the raw blue is $-0.022$ of the
  red.
* **Normalising to the brightest channel throws away brightness**, but the colours' *luminance* is not
  constant: 0.23 at 1 000 K, 0.96 at 6 500 K, 0.49 at 40 000 K. A view that wants brightness to encode luminosity
  (the HR diagram) must scale separately, and a view that does not will make mid-temperature stars look brightest.
* **At 6 500 K the colour is almost white** `(255, 249, 254)`; the whole blue half of the locus is the pale
  band from `#cdd9ff` to `#9eb8ff`, which at the 8-bit resolution of the display is a modest range: a hundred
  steps of temperature above 10 000 K are within 47 levels of blue-white.

## 8. Try it

```sh
# the City uses process_color (state or district), the others kind_color (state or kind with jitter)
./target/release/isotop --demo --view city  --time 12 --width 1200 --renderer cpu --output city.png
./target/release/isotop --demo --view orbit --time 12 --width 1200 --renderer cpu --output orbit.png

# a stopped process is process 9 of the demo (state 'T'); find it in Orbit by its state colour
./target/release/isotop --demo --view orbit --time 12 --width 1600 --renderer cpu --output orbit.png

# tests that pin the palette
cargo test palette_kind_colours_match_their_previous_literals
cargo test mix_blends_each_channel_and_truncates
```

The demo has a stopped process (index 9, `'T'`), a zombie every 41st, and blocked ones, so every state colour
is on screen in a demo frame. Regenerate the figure with `python3 docs/figures/colour-palette.py` from the
repository root.

## 9. Where in the code

* [`src/palette.rs`](https://github.com/tfpickard/isotop/blob/main/src/palette.rs): the constants, `mix`,
  `tint`, `state_color`, `process_color`, `kind_color`, `core_color`, `core_name`, `stone_color`, `DISTRICTS`.
* [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs): `Color`, `Sky`, `Frame::blend`,
  `Frame::ball` (sphere light), `Frame::halo`, `Frame::ray`.
* [`src/render/shared.rs`](https://github.com/tfpickard/isotop/blob/main/src/render/shared.rs): sky colours,
  `SPHERE_AMBIENT`, `SPHERE_LIGHT_*`, `SELECTION_COLOR`.
* [`src/shaders.wgsl`](https://github.com/tfpickard/isotop/blob/main/src/shaders.wgsl) and
  [`src/gpu.rs`](https://github.com/tfpickard/isotop/blob/main/src/gpu.rs) (`COLOR = Rgba8Unorm`).
* [`src/terminal.rs`](https://github.com/tfpickard/isotop/blob/main/src/terminal.rs): the 24-bit overlay colours.

## 10. Variations and open questions

* **A linear-light `mix`.** Decode, blend, encode. Cost: two table lookups per channel. It removes the
  midpoint dip of section 3.3 and changes every golden hash; a candidate for a dedicated PR with before and
  after renders.
* **Separate the stopped state from the session kind.** Give `STATE_STOPPED` a colour outside the jitter
  range of every kind (for example a desaturated cool grey), or draw the state as a ring or hatch. Section 5
  quantifies the collision.
* **Jitter in lightness only along a perceptual axis.** Replace `tint` by a change of $L^*$ at fixed hue so that
  the jitter does not shift saturation when a channel clips at 255.
* **Run the CVD test now.** The planned test of #104 on the *current* palette would fail on the pairs
  above; fixing the stopped/session pair is independent of the `--palette` switch.
* **Blackbody for CPU.** The mockups map CPU to temperature for the HR diagram's stars; what are the perceptual
  consequences of a luminance that is non-monotone in $T$ (section 7)?

## 11. References

* IEC 61966-2-1:1999. *Multimedia systems and equipment - Colour measurement and management - Part 2-1:
  Colour management - Default RGB colour space - sRGB.*
* W3C. 2018. *Web Content Accessibility Guidelines (WCAG) 2.1.* (Relative luminance and contrast ratio,
  success criteria 1.4.3 and 1.4.11.)
* Sharma, G., Wu, W. and Dalal, E. N. 2005. "The CIEDE2000 color-difference formula: implementation notes,
  supplementary test data, and mathematical observations." *Color Research and Application* 30(1): 21-30.
* Machado, G. M., Oliveira, M. M. and Fernandes, L. A. F. 2009. "A physiologically-based model for simulation of
  color vision deficiency." *IEEE Transactions on Visualization and Computer Graphics* 15(6): 1291-1298.
* Brettel, H., Vienot, F. and Mollon, J. D. 1997. "Computerized simulation of color appearance for
  dichromats." *Journal of the Optical Society of America A* 14(10): 2647-2655.
* Okabe, M. and Ito, K. 2008. "Color Universal Design (CUD): how to make figures and presentations that are
  friendly to colorblind people."
* Wyman, C., Sloan, P.-P. and Shirley, P. 2013. "Simple analytic approximations to the CIE XYZ color matching
  functions." *Journal of Computer Graphics Techniques* 2(2): 1-11.
* Kim, Y. S., Cho, B. H., Kim, S. H., Kim, M. J. and Ko, H. S. 2002. "Design of advanced color temperature
  control system for HDTV applications." *Journal of the Korean Physical Society* 41: 865-871 (unverified).
* Harrower, M. and Brewer, C. A. 2003. "ColorBrewer.org: an online tool for selecting colour schemes for maps."
  *The Cartographic Journal* 40(1): 27-37.
