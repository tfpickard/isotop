# Theory GPU and Parity

isotop has two rasterizers for one display list. The CPU path is the reference, deterministic to the bit and the source of the golden hashes; the GPU path, a small wgpu renderer with a WGSL shader, is the default when a hardware adapter exists, because it frees the CPU and scales with resolution. The pair is useful only if they draw the same frame, so the project treats agreement as an invariant with a measurement, a tolerance and a list of known disagreements. This page describes the GPU pipeline as built, the mechanism that keeps the two sets of constants from drifting, the readback that brings pixels and picks back to the CPU (including the one place the code reaches for SSE4.1), why a software adapter is refused, and what "the same frame" can and cannot mean in 8-bit colour.

![The affine depth map from view depth to the GPU z-buffer, and the resolution of a 32-bit depth buffer](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/figures/theory-depth-mapping.svg)

## History

* **Before the GPU.** Real-time 3D on personal computers began as software rasterizers on the CPU; consumer accelerators such as 3dfx's Voodoo (1996) moved triangle rasterization and texturing into hardware. NVIDIA's GeForce 256 (1999) was marketed as the first "GPU", a name chosen to say that transform and lighting had moved too.
* **Shaders and unified APIs.** Programmable shaders, then general-purpose use of the same hardware, followed in the 2000s. Through the 2010s the old global-state APIs gave way to explicit, low-overhead ones: AMD's Mantle (2013) is usually cited as the prompt, Apple's Metal appeared in 2014, Microsoft's Direct3D 12 with Windows 10 in 2015, and Khronos's Vulkan 1.0 in 2016.
* **WebGPU and wgpu.** The W3C's WebGPU standard and its shading language WGSL abstract those APIs for the web. `wgpu` is a Rust implementation of that API with native back ends over Vulkan, Metal and Direct3D 12 (and others), and grew out of the `gfx-rs` project; it is also the engine behind WebGPU in Firefox, which is why a program written against it runs unchanged on the three platforms isotop's `wgpu::Backends::PRIMARY` names. WebGPU shipped in Chrome 113 in 2023 (unverified here).
* **Software as reference.** Graphics APIs have long shipped a slow, exact software implementation to test hardware against: Direct3D has had a reference rasterizer for this purpose, and CPU implementations such as WARP and llvmpipe now exist as fallbacks. Hardware vendors do not promise bit-identical output with each other, only with the specification, and the specification leaves rounding of blends and some fill-rule details to the implementation within bounds. That latitude is why this page has a table of divergences and a tolerance rather than a hash.
* **MOVNTDQA.** The streaming load that `copy_mapped` uses arrived with SSE4.1 in Intel's Penryn generation of processors (2007 to 2008), designed for exactly the case in the source comment, reading from write-combined memory such as a frame buffer.
* isotop treats the older machine as the reference and the newer as the approximation, which is the reverse of the order in which they were built.

## Why the two must agree

`AGENTS.md` states it as an invariant: if you change how anything is rasterized (sky, sphere shading, glow falloff, depth mapping), change `render.rs` and `shaders.wgsl` together, and render the same view with `--renderer cpu` and `--renderer gpu` and compare. Three reasons stand behind it. The picture must not depend on the hardware of the person running it. The pick buffer decides what a click selects ([Theory Picking](Theory-Picking)), and a click must find what the viewer sees. And the CPU is what every test, screenshot and `--output` run uses by default in CI, so a view tuned only against the GPU would be unreviewable.

## The GPU pipeline

`gpu::Gpu` owns a wgpu device and queue, one bind group (a 48-byte uniform `Globals` at binding 0 and the field storage buffer at binding 1), nine render pipelines (one per `Pass`) plus two sky pipelines (`sky_fragment`, and `twilight_fragment` for the `Twilight` backdrop), and per-size render targets. Backends are `wgpu::Backends::PRIMARY` (Vulkan on Linux, Metal on Apple GPUs, DX12 on Windows). Targets are an `Rgba8Unorm` colour texture, an `R32Uint` pick texture and a `Depth32Float` depth texture, all one sample per pixel (no multisampling).

### One pass per primitive family

Every `Item` kind has a pipeline. The *style* flags decide blending, depth and pick writes:

| Pass | Shader entry points | Topology | Colour blend | Pick written | Depth compare / write |
|---|---|---|---|---|---|
| Triangles | `geometry_vertex`, `geometry_fragment` | triangle list | none | yes | `GreaterEqual` / yes |
| Lines | same | line list | none | yes (`NONE`) | `GreaterEqual` / yes |
| Beams | same | line list | alpha blending | no | `Always` / no |
| Stars | same | point list | none | no | `Always` / no |
| Balls | `ball_vertex`, `ball_fragment` | quad (6 vertices per instance) | none | yes | `GreaterEqual` / yes |
| Glows (`Item::Glow` and `Item::Puff`) | `glow_vertex`, `glow_fragment` | quad per instance | alpha blending | no | `Always` / no |
| Glyphs | `glyph_vertex`, `glyph_fragment` | quad per instance | none | no | `Always` / no |
| Splats (`Item::Pick`) | `splat_vertex`, `splat_fragment` | quad per instance | colour writes masked off | yes | `GreaterEqual` / no |
| Fields (`Item::Field`) | `field_vertex`, `field_fragment` | triangle list | none | yes | `GreaterEqual` / yes |
| Sky | `sky_vertex`, `sky_fragment` (`twilight_fragment` for `Backdrop::Twilight`) | one oversized triangle | none | no | `Always` / no |

The sky triangle needs no vertex buffer: `corner = ((i << 1) & 2, i & 2)` for vertex index $i = 0, 1, 2$ gives $(0,0), (2,0), (0,2)$, and $2\cdot\text{corner} - 1$ is $(-1,-1), (3,-1), (-1,3)$, which covers the clip square. Opaque primitives write colour, pick and depth together; the blended ones write only colour. This is the same split as the CPU's: lines clear picks and beams do not, spheres and triangles are pickable, glows are not.

**Batching preserves order.** `Gpu::render` walks `frame.items` once, routes each item into one of six vertex streams (`vertices`, `balls`, `glows`, `glyphs`, `splats`, `field_vertices`) and records a batch `(Pass, range)`, extending the previous batch while consecutive items use the same pass. The batches are drawn in list order, and within a draw call primitives are processed in order, so painter's order and the blend order of translucent items survive. The price is that alternating primitive kinds produce many small draws; issue [#87](https://github.com/tfpickard/isotop/issues/87) (A28) proposes layered blended passes and pipelined readback.

**Instance data.** Spheres, glows, glyphs and pick discs are instances of a six-vertex quad expanded in the vertex shader from the constant array `CORNERS`. Each instance struct is tightly packed and its size and field offsets are pinned by compile-time assertions that must match the WGSL `struct` of the same name: `Globals` 48 bytes, `Vertex` 20, `Ball` 32, `Glow` 20, `Glyph` 32, `Splat` 20, `FieldVertex` 24. A triangle is three `Vertex` entries of position (two floats), depth (one float), colour (4 normalised bytes) and pick (`u32`); there is no index buffer. Streams are reused across frames and grown to the next power of two (minimum 4096 bytes).

### The sphere on the GPU

`ball_vertex` expands a quad of half-size `radius + SELECTION_RING_WIDTH + 1` around the centre, at constant clip depth 0.5. `ball_fragment` then does exactly what `Frame::ball` does: it samples at the integer pixel coordinates, `n = (floor(position.xy) - center) / radius`, tests $d \le 1$, computes $n_z = \sqrt{1-d}$ and the shared light formula, and *writes the depth itself* through `@builtin(frag_depth)` as `normalized(center.z + nz * world)`. Because the shader writes depth, early depth testing is unavailable for that pass, which is the standard price of an impostor. The derivation is in [Theory Scene and Display List](Theory-Scene-and-Display-List).

### Depth: an affine map per frame

The vertex stage maps pixels to clip space, $x = 2X/W - 1$, $y = 1 - 2Y/H$, $w = 1$ (the projection has no perspective divide; see [Theory Camera and Projection](Theory-Camera-and-Projection)), and depth through

$$
z_{\text{buf}} = \operatorname{clamp}\!\Bigl(0.01 + 0.98\,\frac{D - D_{\min}}{D_{\max} - D_{\min}},\; 0,\; 1\Bigr).
$$

`Gpu::render` computes $D_{\min}$ and $D_{\max}$ by tracking the depth of triangle vertices, line endpoints plus `LINE_DEPTH_BIAS`, sphere centres $\pm$ their world radius and pick discs, and uploads $D_{\min}$ and $1/\max(D_{\max} - D_{\min},\, 10^{-3})$ in `globals.depth` (an empty range falls back to $[0, 1]$). The buffer is cleared to 0 and tested with `GreaterEqual`, so larger $D$, nearer, wins; ties pass, as on the CPU. The 0.01 and 0.99 margins keep every solid fragment strictly inside the clip range.

Because the map is one increasing affine function applied to every depth in the frame, comparing mapped values orders any two fragments exactly as comparing raw $D$ does, up to rounding. The remaining question is whether 32-bit floats resolve what the scene needs. The step between adjacent representable values of `Depth32Float` is $2^{-24}$ for $z \in [0.5, 1)$ and $2^{-30}$ near 0.01, so the smallest depth difference resolved is about $2^{-24}\,(D_{\max} - D_{\min})/0.98$ world units. The right panel of the figure plots it: the 0.02-unit line bias equals one step only when a frame spans about $3.3 \times 10^5$ world units, and for a range of $10^3$ it is some 330 steps. Depth precision is not what limits parity; fill rules and blending are.

### Fields

The field pass samples one storage buffer by hand, with no hardware sampler, so that it can apply the same clamped-uv, `floor(bilerp + 0.5)` formula as the CPU's `Frame::field_triangle` (`field_texel` and `field_fragment` in the shader). `Gpu::store_fields` rewrites only the slots whose field was shaded again that frame (`StoragePlan`, from `render/field.rs`), and grows the buffer if a field does not fit. The pick written is the owner of the nearest node. The details of the field primitive are on [Theory Scene and Display List](Theory-Scene-and-Display-List).

### The sky

`twilight_fragment` does the same for the three-stop `Twilight` gradient (`TWILIGHT_TOP`, `TWILIGHT_MIDDLE`, `TWILIGHT_HORIZON`, break at `TWILIGHT_KNEE = 0.6`). `sky_fragment` reproduces `paint_sky`: the backdrop discriminant arrives as a float in `globals.sky.x` and each test is the midpoint between neighbouring discriminants, the base colour is the shared gradient, the haze density is the same product of two sinusoids, and the result is `floor(color)/255`, which an `Rgba8Unorm` target stores as exactly the integer. Using `floor(position.x) / size.x` reproduces the CPU's `x / w`.

## Shared constants: one source, two languages

A constant that both rasterizers use is declared once in `render/shared.rs` through the `shared!` macro, which defines the Rust `const` and lists it in `CONSTANTS` with a `Wgsl` value (`U32`, `F32` or `Vec3`). `shared::prelude()` renders that table to WGSL source, one declaration per line:

```text
// Generated from render::shared; do not edit.
const NONE: u32 = 4294967295u;
const HAZE_FREQUENCY_A: f32 = 7.3;
const SPACE_BASE: vec3<f32> = vec3<f32>(5.0, 8.0, 18.0);
```

and `gpu::shader_source()` is `format!("{}{}", shared::prelude(), include_str!("shaders.wgsl"))`. Floats are printed with `{:?}`, which is the shortest text that parses back to the same `f32` and always has a decimal point or exponent, as WGSL needs; the rule "never write a shared value in `shaders.wgsl`" is enforced by tests: every constant appears exactly once in the prelude, literals print exactly, round-trip to the same bits, and the shader source must not contain the literals `0.65`, `7.3`, `4294967295`, `0xffffffff`, `237.0` or `0.25`. The constants cover the pick sentinel, the five backdrops and their colours, the haze frequencies, the sphere light, the sphere edge and selection ring, the line depth bias and the glyph cell size. The compile-time layout assertions play the same role for the GPU structs.

## Readback

The terminal needs pixels in CPU memory, so every GPU frame ends with a copy back. `Gpu::render` encodes copy commands into one readback buffer: the whole colour texture (`row = width * 4` rounded up to `COPY_BYTES_PER_ROW_ALIGNMENT`, 256 bytes), and a **window of the pick texture** around the pointer, `PICK_WINDOW = 160` pixels square, with `PICK_ROW = 768` bytes per row. At 1920 by 1080 that is $7680 \times 1080 = 8{,}294{,}400$ bytes of colour (exactly 7680 bytes per row, no padding) plus $768 \times 160 = 122{,}880$ of picks, an 8.4 MB buffer. It submits, maps the buffer asynchronously and blocks on `device.poll(wait_indefinitely)`, so a frame is strictly sequential: record, draw, wait, copy. The pointer window is why picks are cheap: hover and clicks never look further than a few cells, so only 25,600 of 2,073,600 pick texels (1.2%) are read; the rest of `frame.picks` is `NONE`. Anything that needs picks away from the pointer must rasterize on the CPU, as the convention in `docs/VIEW_CONVENTIONS.md` says.

The mapped memory is then copied into a scratch `Vec<u8>` and converted from RGBA to RGB into `frame.pixels`. That intermediate copy is `copy_mapped`:

* Readback memory can be uncached or write-combined (the code names Intel integrated GPUs), where ordinary loads are slow because every read bypasses the cache. On `x86_64`, when `sse4.1` is detected at run time and the source is 16-byte aligned, `copy_mapped` reads it with `_mm_stream_load_si128` (the `MOVNTDQA` instruction) in 16-byte chunks into cached memory and stores with `_mm_storeu_si128`, finishing any tail with an ordinary copy. The comment in the source claims it is hundreds of times faster than reading the mapped memory piecemeal during the conversion; I have not measured it.
* Everywhere else (Apple Silicon, unaligned buffers, no SSE4.1) it is `extend_from_slice`.
* It is one of the project's small `unsafe` blocks: the `// SAFETY:` comment states that SSE4.1 is present, the source aligned, every load within `whole` bytes, the destination reserved, and its length set only after writing.

The GPU's pixels then go through the same `Terminal::present` as the CPU's ([Theory Terminal Graphics](Theory-Terminal-Graphics)).

## Which adapter, and when to give up

`Gpu::new(power)` requests the preferred adapter for `--gpu-power` (`high` by default, `low` to leave a discrete GPU asleep) and **refuses software adapters**: if `device_type` is `Cpu` it returns an error naming the adapter. The reason is in the doc comment: a CPU emulation of a GPU (llvmpipe, WARP) is slower than isotop's own CPU rasterizer. CI has no hardware GPU on Linux. The error is data: under `--renderer auto` (the default) `App::choose_renderer` falls back to the CPU and the status strip says why; on this wiki's build machine, which has no GPU, it reads `cpu (no GPU adapter: No suitable graphics adapter found; ...)`. Under `--renderer gpu` the error stops the run. A GPU failure *during* a run (any uncaptured wgpu error, or a failed map) discards the GPU, rasterizes that frame on the CPU, and the status strip reads `cpu (GPU failed: ...)` for the rest of the session. Required limits are the downlevel defaults raised to the adapter's resolution limits so that the widest frame (`--width` up to 2560) fits.

## Parity: measuring it

```sh
isotop --demo --view <name> --time <T> --width 960 --renderer cpu --output cpu.png
isotop --demo --view <name> --time <T> --width 960 --renderer gpu --output gpu.png
python3 scripts/compare_png.py cpu.png gpu.png
```

`scripts/compare_png.py` decodes both PNGs (standard library only: all five scanline filters, 8-bit RGB or RGBA, alpha ignored), and prints one line: `mean M | share S % | max X | size WxH`. With per-channel absolute differences $\delta$:

* **mean** $M = \dfrac{\sum \lvert \delta \rvert}{3\,W H}$, the mean absolute difference per channel on the 0 to 255 scale;
* **share** $S$, the percentage of *pixels* where any channel differs by more than `--levels` (default 2);
* **max** $X$, the largest single-channel difference;
* the exit status is 1 if the sizes differ or $M$ or $S$ exceeds `--max-mean` / `--max-share`, 2 if a file cannot be decoded.

The recorded tolerance (`docs/VIEW_CONVENTIONS.md`, "CPU/GPU parity"):

| Limit | Mean | Share | Views |
|---|---|---|---|
| Default | $\le 0.25$ | $\le 0.25\,\%$ | every view not named below, including every new one |
| Line-dominant | $\le 0.5$ | $\le 1.0\,\%$ | city, ripple, cores, cells, globe |

At 960 by 540 (518,400 pixels) the default share permits about 1,300 pixels more than two levels off, and the line-dominant one about 5,200. A view PR never loosens the numbers, and a new view may not use the line-dominant limit: it draws its one-pixel strokes with `Frame::hairline` (triangles, which both rasterizers fill alike), and a row with `link_strokes` draws its scene links the same way. The numbers recorded on an Apple GPU (Metal) at `--demo --time 12 --width 960`:

| View | Mean | Share % | Max | Verdict |
|---|---|---|---|---|
| city | 0.1194 | 0.7413 | 160 | line-dominant |
| orbit | 0.3914 | 1.2724 | 235 | exceeds both (share) |
| ripple | 0.1891 | 0.9053 | 113 | line-dominant |
| flow | 2.9492 | 10.7836 | 225 | exceeds both |
| cores | 0.1900 | 0.2905 | 231 | line-dominant |
| cells | 0.2216 | 0.2602 | 238 | line-dominant |
| strata | 0.5268 | 0.5766 | 241 | exceeds both (mean) |
| globe | 0.3366 | 0.4558 | 185 | line-dominant |
| reef | 0.0485 | 0.1524 | 123 | default |
| matrix | 0.0335 | 0.0000 | 2 | default |
| coop | 0.0149 | 0.0237 | 192 | default |
| murmuration | 0.0000 | 0.0000 | 1 | default |

Orbit, Flow and Strata exceed even the line-dominant limit; the conventions call them known parity debt of views that predate the tolerance, to be fixed in the rasterizer, not in a view. CI measures parity on the `macos-15` runner for every view (`parity.txt`, an artifact) with `continue-on-error`: it is evidence, not a gate. There is no automated GPU test on Linux; the one GPU test in the tree (`pick_splats_and_thing_picks_survive_the_gpu_readback`) is `#[ignore]` and needs hardware, and asserts that for a small hand-built frame the GPU's colour *and* pick buffers equal the CPU's exactly.

## Known divergences

Issue [#83](https://github.com/tfpickard/isotop/issues/83) (A24, open, "changes pixels, needs decision") is the audit. Each row below I checked against the current source; where the GPU side is a hardware rule I rely on the issue and `docs/VIEW_CONVENTIONS.md`.

| Feature | CPU | GPU | Effect |
|---|---|---|---|
| Glow and beam blending (`Item::Puff` rounds on both, by design: the CPU's `blend_rounded`) | $c + (t - c)\,w$ in `f32`, converted with `as u8` (truncation), weight unclamped | normalised-unorm alpha blending (rounds, clamps); beam alpha quantised to 8 bits as `(weight.clamp(0,1) * 255) as u8` | faint beams do nothing (or darken by one level) on the CPU and add one level on the GPU; weights below 1/255 vanish on the GPU; Flow's thousands of beam segments accumulate the bias |
| Lines and beams | DDA, truncation, both endpoints | hardware line rule (diamond exit; the last pixel is dropped) | up to a pixel of offset in line-heavy views; the reason for the line-dominant limit |
| Triangle edges | pixel centre with $\lambda \ge 0$ on every edge, shared-edge pixels drawn twice, later wins | hardware fill rule (top-left, per the issue) | colour and pick ownership differ along shared edges |
| Haze | added only when `Sky::hazy()` (a channel above 0.5) | always added | the GPU can be one level brighter on a nearly clear sky |
| Glyph origin | truncated with `as i64` | the float origin is subtracted before `floor` | differ at fractional or negative origins |
| Stars | depth-tested write of `BACKGROUND` | compare `Always`, no depth write | agree only because stars come first in the list |

Two further things I found while reading and which the issue does not list: the shared constant `SPHERE_EDGE` widens only the scan box of an *unselected* sphere on both rasterizers (nothing is drawn in the margin, despite the doc comment's "outline"), and the CPU's weight is `strength * (1-d)^2` unclamped while a GPU unorm source alpha is clamped to $[0,1]$, so a glow with `strength` above 1 would blend differently; I did not check whether any view emits one.

The proposed fix (A24) rounds and quantises on the CPU exactly as the GPU does, adopts the top-left rule and floors glyph origins, and then tightens the tolerance; it changes CPU pixels slightly in every view, which is why it awaits a decision, and why the golden hashes in `tests/golden.txt` would be re-blessed with before-and-after renders.

## Reading the picture

A frame from the GPU is the same *content* as one from the CPU, rasterized under different rounding. Differences are at the level of single pixels along line-drawn structure and at one or two levels of brightness in translucent regions. Anything a viewer can read at the scale of a body or a trail is common to both; anything they can read at the scale of a pixel (a one-pixel orbit trace, a faint beam, a seam between two faces) may not be.

## Where in the code

| What | Where |
|---|---|
| `Gpu`, `Gpu::new`, `Gpu::render`, passes and styles, readback, `copy_mapped`, struct layouts | [`src/gpu.rs`](https://github.com/tfpickard/isotop/blob/main/src/gpu.rs) |
| Shader entry points, `normalized`, `clip`, `ball_fragment`, `glow_fragment`, `sky_fragment` | [`src/shaders.wgsl`](https://github.com/tfpickard/isotop/blob/main/src/shaders.wgsl) |
| Shared constants and `prelude` | [`src/render/shared.rs`](https://github.com/tfpickard/isotop/blob/main/src/render/shared.rs) |
| Renderer choice and fallback | [`src/main.rs`](https://github.com/tfpickard/isotop/blob/main/src/main.rs): `App::choose_renderer`, `App::render` |
| Comparison script | [`scripts/compare_png.py`](https://github.com/tfpickard/isotop/blob/main/scripts/compare_png.py) |
| Tolerance and measurements | [`docs/VIEW_CONVENTIONS.md`](https://github.com/tfpickard/isotop/blob/main/docs/VIEW_CONVENTIONS.md), section "CPU/GPU parity" |
| CI parity job | [`.github/workflows/ci.yml`](https://github.com/tfpickard/isotop/blob/main/.github/workflows/ci.yml) |

Figure: [`docs/figures/theory-depth-mapping.py`](https://github.com/tfpickard/isotop/blob/main/docs/figures/theory-depth-mapping.py).

## References

* T. Akenine-Moller, E. Haines and N. Hoffman, *Real-Time Rendering*, 4th ed., CRC Press, 2018 (z-buffers, blending, rasterization rules).
* W3C, *WebGPU Shading Language* specification (the language of `shaders.wgsl`).
* *Intel 64 and IA-32 Architectures Software Developer's Manual*, instruction reference for `MOVNTDQA` (streaming load).
* isotop issues [#83](https://github.com/tfpickard/isotop/issues/83), [#85](https://github.com/tfpickard/isotop/issues/85), [#87](https://github.com/tfpickard/isotop/issues/87).
