# src/render

## OVERVIEW
Rasterizer primitives behind `render.rs`: shared CPU/GPU constants, strokes, fields. Own file because every change
here must keep two backends (`Frame::rasterize` and `gpu.rs` with `shaders.wgsl`) pixel-equivalent.

## WHERE TO LOOK
| Task | File | Symbol / notes |
|---|---|---|
| Constant both backends need | `shared.rs` | `shared!` macro, `CONSTANTS`, `prelude` writes them into the WGSL source |
| World light direction | `shared.rs` | `LIGHT` (Rust only, see below) |
| Wide lines | `strokes.rs` | `Frame::ribbon`, `Frame::tube`, `Width`, `Dash`, `ring_frames`, `tube_faces` |
| Grid of colours in world space | `field.rs` | `Field`, `Placement`, `Relief`, `Shading`, `Frame::field`, `Item::Field` |
| Shaded-field caching | `field.rs` | `ShadedField`, recycled through `Buffers` by `(key, revision)` |
| GPU field storage | `field.rs` | `StorageLayout` plans what the storage buffer rewrites |
| CPU field spans | `field.rs` | `Frame::field_triangle` |
| Picking, items, scene | `../render.rs` | `Frame`, `Item`, `Stage`, `Scene`, `THING` pick flag |

## CONVENTIONS
- Declare a shared constant once: `shared! { U32|F32|Vec3 NAME: T = v; }`. Floats print with `{:?}` so WGSL gets a decimal point.
- `LIGHT` is Rust-only: shading is baked into colours before the backends see them. It is not in the shared list.
- Strokes are `Item::Triangle`s only. Each piece is at most 32 px an edge, so a triangle box is at most 64 x 64 px.
- Field sampling is one formula for both backends: clamped uv in node units, bilinear interpolation, `floor(bilerp + 0.5)`
  per channel, the owner of the nearest node as the pick. Change CPU and `shaders.wgsl` together.
- Grid simulations draw through `Frame::field`, not one triangle pair per cell.
- The GPU field pass samples one storage buffer by hand (no hardware sampler).

## ANTI-PATTERNS
- "Never write one of these numbers into `shaders.wgsl` directly" (`shared.rs`); the test
  `shader_source_does_not_repeat_shared_literals` enforces it.
- Do not move `LIGHT` into `shared!`.
- Do not emit a stroke piece longer than 32 px an edge.
- Do not change rasterization in `render.rs` without `shaders.wgsl` (see root Invariants for the parity check).
- Do not stack `Item::Glow` for density; use `Item::Puff`, which rounds the same on both backends.

## COMMANDS
- `cargo test render::`
- `./target/release/isotop --demo --view <name> --renderer cpu --width 960 --time 12 --output /tmp/cpu.png`, the same with
  `--renderer gpu`, then `python3 scripts/compare_png.py /tmp/cpu.png /tmp/gpu.png`.
