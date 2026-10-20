# src/render

## OVERVIEW
The submodules of `render.rs`: the camera, the display list, the CPU rasterizer, the world-space emitters, the
`Stage` a view draws into, the `Scene`, and the primitives (shared CPU/GPU constants, strokes, fields, stroked
text). Own file because every change to how something is rasterized must keep two backends (`Frame::rasterize` in
`raster.rs`, and `gpu.rs` with `shaders.wgsl`) pixel-equivalent.

## WHERE TO LOOK
| Task | File | Symbol / notes |
|---|---|---|
| Module root and re-exports | `../render.rs` | `Color`, `Point`, `Links`; re-exports keep `crate::render::{Frame, Camera, Item, Scene, ...}` working |
| Projection, pan, zoom, tilt | `camera.rs` | `Camera`, `SCALE`, `ISOMETRIC`, `LOWEST_PITCH`, `ORIGIN_Y` |
| Display list, picking | `frame.rs` | `Item`, `Frame`, `Thing`, `Target`, `Card`, `Buffers`, `THING`, `NONE`, `thing_pick`, `target_near`, `pick_near` |
| CPU rasterizer | `raster.rs` | `Frame::rasterize` and its primitives; `gpu.rs` mirrors it |
| World-space emitters | `draw.rs` | `Frame::project`, `facet`, `quad`, `line`, `hairline`, `sphere`, `glow`, `beam`; `cover`, `similar`, `glows` |
| What a view reads and writes | `stage.rs` | `Stage`, `RenderInput`, `FrameClock`, `MAX_DT`, `TourTarget`, `Title` |
| Frame-wide state | `scene.rs`, `scene/tests.rs` | `Scene` (no per-view field), `fit`, `draw_links`, the exit flashes |
| Constant both backends need | `shared.rs` | `shared!` macro, `CONSTANTS`, `prelude` writes them into the WGSL source |
| World light direction | `shared.rs` | `LIGHT` (Rust only, see below) |
| Wide lines | `strokes.rs` | `Frame::ribbon`, `Frame::tube`, `Width`, `Dash`, `ring_frames`, `tube_faces` |
| Grid of colours in world space | `field.rs` | `Field`, `Placement`, `Relief`, `Shading`, `Frame::field`, `Item::Field` |
| Shaded-field caching | `field.rs` | `ShadedField`, recycled through `Buffers` by `(key, revision)` |
| GPU field storage | `field.rs` | `StorageLayout` plans what the storage buffer rewrites |
| CPU field spans | `field.rs` | `Frame::field_triangle` |
| Text drawn into the scene | `text.rs` | `Frame::stroke_text` (Hershey glyphs from `../hershey.rs`), `Align`; no new `Item` |

## CONVENTIONS
- Declare a shared constant once: `shared! { U32|F32|Vec3 NAME: T = v; }`. Floats print with `{:?}` so WGSL gets a decimal point.
- `LIGHT` is Rust-only: shading is baked into colours before the backends see them. It is not in the shared list.
- Strokes are `Item::Triangle`s only. Each piece is at most 32 px an edge, so a triangle box is at most 64 x 64 px.
- `Frame::stroke_text` builds on `Frame::line` and `Frame::ribbon`, so the GPU needs nothing new. It is unpickable,
  and process text reaches it only after cleaning (as `terminal::SafeText::new` does).
- Field sampling is one formula for both backends: clamped uv in node units, bilinear interpolation, `floor(bilerp + 0.5)`
  per channel, the owner of the nearest node as the pick. Change `field.rs` and `shaders.wgsl` together.
- Grid simulations draw through `Frame::field`, not one triangle pair per cell.
- The GPU field pass samples one storage buffer by hand (no hardware sampler).
- `Scene` holds frame-wide state only; a new field needs a doc comment saying why and a line in
  `scene_has_no_view_specific_fields`.

## ANTI-PATTERNS
- "Never write one of these numbers into `shaders.wgsl` directly" (`shared.rs`); the test
  `shader_source_does_not_repeat_shared_literals` enforces it.
- Don't move `LIGHT` into `shared!`.
- Don't emit a stroke piece longer than 32 px an edge.
- Don't change `raster.rs` without `gpu.rs` and `shaders.wgsl` (see root Invariants for the parity check).
- Don't stack `Item::Glow` for density; use `Item::Puff`, which rounds the same on both backends.

## COMMANDS
- `cargo test render::`
- `./target/release/isotop --demo --view <name> --renderer cpu --width 960 --time 12 --output /tmp/cpu.png`, the same with
  `--renderer gpu`, then `python3 scripts/compare_png.py /tmp/cpu.png /tmp/gpu.png`.
