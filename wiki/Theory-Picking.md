# Theory Picking

Picking answers one question: what is under the pointer? isotop answers it the way many renderers do, by drawing it. Next to the colour image the rasterizer fills a second image of the same size, the *pick buffer*, in which every pixel holds not a colour but the identity of whatever won the depth test there. A click is then a lookup. This page covers that buffer's format (which carries two kinds of identity in one 32-bit word), the rules for which primitives write it, the search that turns an imprecise terminal pointer into a hit, and the places where the machinery and the picture can disagree.

## History

* **The pointing device.** Selecting by pointing needs a pointer. Douglas Engelbart's group at the Stanford Research Institute built the first mouse in the mid-1960s, a wooden shell with two wheels, and demonstrated it in 1968; the terminal mouse reports that isotop reads are descendants of that arrangement filtered through the X Window System.
* **Selection by drawing.** Early graphics libraries answered "what is under the pointer" with a separate mode. OpenGL 1.0 (1992) offered `GL_SELECT` with a name stack: draw the scene in a special mode and read back the names of what fell in a small clipping region. It was deprecated with the rest of the fixed-function pipeline. The replacement that programmers reached for is the one on this page: render identifiers as colours into an off-screen image and read the pixel.
* **The item buffer.** The technique has an older name. Hank Weghorst, Gary Hooper and Donald Greenberg, "Improved Computational Methods for Ray Tracing", *ACM Transactions on Graphics* 3(1), 1984, 52-69, introduced the *item buffer*: a pre-pass that records, per pixel, which object is visible, used to skip most of the ray tests. Picking by ID buffer is the same image put to a different use.
* **Fitts.** Paul Fitts's 1954 paper treated the hand as a communication channel with a capacity in bits, and borrowed its form from Shannon's channel-capacity theorem; the movement time to a target grows with the logarithm of distance over width. Later work (MacKenzie, 1992, among others) adapted the formula for pointing devices, and it is the reason a hit zone larger than the visible target is a standard remedy. The experiments used a stylus tapping between metal plates.
* **Mouse reports in terminals.** The cell-based mouse reports of xterm (X10 and X11 modes, then the SGR encoding, mode 1006) predate pixel reports; mode 1016 reports pixels and is the newest. Which terminal added it first is not established here (unverified).
* On the page below the pointer is about 13 bits and the frame about 21. Fitts's subjects were not asked to supply the rest.

## Theory: object-ID buffers and an uncertain pointer

There are two standard ways to hit-test a scene. *Ray casting* intersects a ray through the pointer with every object, which needs the scene's geometry in a queryable form and a depth-resolution rule. *ID buffering* instead renders the scene a second time, or in the same pass, with each object's identifier as its "colour"; the visible object at a pixel is, by construction, the one the viewer sees there, with occlusion, depth ties and silhouettes already resolved by the very code that drew them. Its costs are memory (here 4 bytes per pixel) and that the answer is quantised to the pixel grid and limited to what was rasterized as opaque. For isotop, where most bodies are impostors with no mesh to cast against, the second approach is the only reasonable one.

The pointer is the other half. A terminal reports the mouse in character cells unless it supports SGR-Pixels reporting (mode 1016). A cell report carries only $\log_2(\text{columns} \times \text{rows})$ bits of position, about 13 bits for a 200 by 50 grid, against about 21 bits for a 1920 by 1080 frame. The pointer is therefore a *region*, not a point, and a small body can be smaller than the uncertainty. The standard remedy is to treat the target as larger than it is: in Fitts's law terms (Fitts 1954), the effective target width grows by the search diameter, which lowers the index of difficulty of acquiring it. isotop does exactly that, by searching a disc.

## The code

### What a pick value is

`Frame::picks` is a `Vec<u32>` of $W \times H$ values:

| Value | Meaning |
|---|---|
| `NONE` = `u32::MAX` | nothing pickable here |
| $v < 2^{31}$ | a **process**: $v$ indexes `Frame::identities` |
| $v \,\&\, \text{THING} \ne 0$, where `THING` $= 2^{31}$ | a **thing** (not a process): $v \,\&\, \lnot\text{THING}$ indexes `Frame::things` |

`Scene::render` fills `identities` with the processes it admitted (in `Identity` order) before the view draws, and (in debug builds) asserts that their count stays below `THING`. A process item carries its index; a view that draws something that is not a process (a grave in the Necropolis view is the shipped example; the interrupt lines, open files and core blocks that views could draw are others) asks `Frame::thing_pick(Thing { kind, key })`, which pushes the thing onto `frame.things` and returns `(index) | THING`. Rules from `docs/VIEW_CONVENTIONS.md` and the source: call it once per thing per frame and give every item of the thing that value; `kind` is a short lowercase noun the inspector names it by; the key stays the same across frames; and *never mint a synthetic `Identity`*. The thing index space stays below `THING - 1`, so a thing value can never equal `NONE`. `Frame::target(pick)` resolves a value to a `Target::Process(Identity)` or `Target::Thing(Thing)`, or `None` for `NONE` or an index past the table.

Indices, not identities, are stored because a frame has at most a few thousand processes and a pick buffer is read and written per pixel; the table lookup happens once per query.

### Which primitives write it

| Item | Pick behaviour |
|---|---|
| `Triangle` | writes its `pick` where it passes the depth test (decorative triangles carry `NONE`) |
| `Sphere` | writes its `pick` over the disc *and* over the selection ring |
| `Line` | writes **`NONE`** where it passes the depth test: lines clear picking |
| `Star` | writes `NONE` |
| `Field` | writes the owner of the nearest node of its field (`NONE` where the grid has no owner) |
| `Beam`, `Glow`, `Puff`, `Glyph` | never touch the pick buffer |
| `Pick` | writes only picks, over a disc, where its depth is at least the stored depth; no colour, no depth |

The semantics are "what you see is what you pick": a line is opaque on screen and unpickable, so it masks whatever it covers; blended content is translucent, so what is underneath stays pickable. Two corollaries come straight from this table. A decorative one-pixel stroke drawn across a body makes those pixels unclickable, so decoration near bodies is drawn as `Beam`s (which never touch picks) or behind the body in depth. And translucent or splatted content that *should* be clickable needs an `Item::Pick`, recorded **after** the geometry it lies over: it writes no depth, so geometry recorded later covers it wherever that geometry passes the depth test, and it is depth-tested at its own `depth` like a sphere. The GPU rasterizer reproduces all of this with a second colour attachment of format `R32Uint` and per-pipeline write masks ([Theory GPU and Parity](Theory-GPU-and-Parity)); the `pick` attribute is flat-interpolated so a triangle writes a single value.

### From pointer to hit: Frame::nearest

The pointer arrives as a frame-space point $(x, y)$ and a radius $\rho$ (below). `Frame::target_near` returns the process or thing at the *nearest pickable pixel within $\rho$*:

```text
reach = ceil(rho);  (cx, cy) = (trunc(x), trunc(y))
best = none;  best_distance = rho^2
for py in cy-reach ..= cy+reach, px in cx-reach ..= cx+reach  (clamped to the frame):
    d = (px + 0.5 - x)^2 + (py + 0.5 - y)^2
    if accept(picks[py][px]) && d <= best_distance:  best = pick;  best_distance = d
```

The distance is measured from the pointer to the pixel *centre*, and a pixel qualifies if it is no farther than $\rho$; because the comparison is `<=`, among pixels equally near the **last in row order** wins. `accept` is "is not `NONE`" for `target_near` and "is a process" for `pick_near`, which still exists for process-only callers and tests. The search reads $(2\lceil\rho\rceil + 1)^2$ pixels, 1,681 at $\rho = 20$, a negligible cost even though it runs up to three times per frame (hover highlight, the hover tag, and any click).

Note what it does *not* do: it picks the nearest *pixel*, not the nearest *object*. A large body whose nearest pixel lies 12 px away loses to a small one whose single pixel lies 11 px away. With the pointer on a body's own pixel, that pixel is within 0.71 px, so the body wins unless a neighbouring pixel of something else is nearer still or exactly as near. The tie rule is the one the tests pin down (`target_near_finds_the_nearest_thing_or_process`).

### The radius and cell-coarse input

`Layout` (in `app/screen.rs`) holds the frame size, the terminal's cell grid, and, when the terminal reports its window size, the pixels per cell. Frame pixels per cell are $w = W/\text{columns}$ and $h = H/\text{scene rows}$ (the frame is scaled to fill the scene's cells; the five bottom rows are the status panel). A mouse report becomes a frame-space point as follows:

| Report | Point | Radius $\rho$ |
|---|---|---|
| cell (no SGR-Pixels, or unknown window size) | the centre of the cell, $((c + \tfrac12)w,\; (r + \tfrac12)h)$ | $\max(w, h)$: one cell |
| pixel (SGR-Pixels, mode 1016, with the cell size known) | $x \cdot W / (\text{columns} \cdot c_w)$ and likewise for $y$, scaled from window pixels to frame pixels | $0.6\,\max(w, h)$ |

So a pixel report still gets a search disc of 0.6 of a cell, since a body can be a few pixels across and a hand is not a pixel-precise instrument. The 1016 mode is requested at startup only if the capability probe says the terminal knows it ([Theory Terminal Graphics](Theory-Terminal-Graphics)). As an example from the capture used for these pages, a 120 by 36 terminal with 8 by 16 px cells and a 320-pixel frame has $w = 2.67$ and $h = 5.32$ frame pixels per cell, so a cell report searches a disc of 5.3 frame pixels.

### Hover, click and the one-frame lag

After each rasterization `App::hover_over` runs `target_near` at the last pointer position and stores the result as `hovered` (a process) or `hovered_thing`; the *next* render highlights it (`Scene::highlight`, `RenderInput::thing`). Hover therefore trails the pointer by one frame. `App::overlay` queries again for the hover tag. A left click with the pointer in the scene and not over a popup calls `target_near` on the frame just drawn and selects the process or thing, or, on empty space, arms a dismissal that fires on button release unless the press became a drag.

On the GPU only a window of the pick texture is read back, `PICK_WINDOW = 160` px square centred on the pointer *as it was when that frame was rendered*, and the rest of `frame.picks` is `NONE`. A click uses the frame drawn from the pointer's position at render time. With $\rho$ at most a few cells and a window half-width of 80 px, hover and clicks always find what they need unless the pointer moves a large fraction of a window between the render and the click; this is a consequence of the code, not something I have seen happen.

### Things and cards

A selected thing opens a *card*, built on demand by `ViewModule::card(thing, snapshot)` from the view's own retained state, never one per thing per frame, and its text is untrusted like process text ([Theory Text Safety](Theory-Text-Safety)). A card returns `None` once the view no longer shows the thing, and the popup then says `<kind> no longer shown`. `Stage::thing` is the selected thing or, failing that, the hovered one, for the view to highlight. Of the shipped views, only Necropolis draws things so far (graves and their stones); the others pick processes.

## What the tests pin down

`depth_buffer_picks_the_frontmost_object`, `things_and_processes_share_the_pick_buffer_without_colliding` (thing 0 is not process 0, and the same key under different kinds stays distinct), `pick_near_still_returns_only_processes`, `target_near_finds_the_nearest_thing_or_process`, `a_pick_splat_writes_picks_but_no_pixels_or_depth` (81 lattice points for $r=5$, Gauss's circle count), `a_pick_splat_behind_a_triangle_is_not_picked`, and `cell_sized_picking_selects_the_nearest_small_object` are in `render.rs`; the view harness property `ValidPicks` checks for every registered view that every pick names a process of the frame; the golden hashes cover the pick buffer of the CPU render of every view.

## Reading the picture

* **You can pick what is opaque.** A glow, a beam, a haze or a trail is not pickable on its own; a view that wants it so draws an `Item::Pick` over its geometry (Murmuration does, under its splatted flock).
* **A line can hide a body.** Where a line passes the depth test it replaces the pick with `NONE`; the radius search usually finds the body beside it, which is the practical reason the search exists.
* **Radius makes the hit zone bigger than the body.** For a 3.5-pixel body (`MIN_BODY_PX`) the effective target is a disc of $3.5 + \rho$ pixels. Clicking near an empty region next to a body selects the body.
* **Hover is a frame behind** and the GPU pick buffer is blind away from the pointer; neither is visible in use, both matter to anyone writing a test that picks elsewhere (such a test rasterizes on the CPU).
* **A pick is of a frame**, not of the world: indices are per frame, which is why a selection is stored as an `Identity` or a `Thing`, never as a pick value.

## Where in the code

| What | Where |
|---|---|
| `NONE`, `THING`, `Frame::picks`, `identities`, `things`, `thing_pick`, `target`, `pick_near`, `target_near`, `nearest`, `Item::Pick`, `pick_disc`, `Target`, `Thing`, `Card` | [`src/render.rs`](https://github.com/tfpickard/isotop/blob/main/src/render.rs) |
| `NONE` shared with the shader | [`src/render/shared.rs`](https://github.com/tfpickard/isotop/blob/main/src/render/shared.rs) |
| Pick attachment, write masks, `PICK_WINDOW`, readback window | [`src/gpu.rs`](https://github.com/tfpickard/isotop/blob/main/src/gpu.rs), [`src/shaders.wgsl`](https://github.com/tfpickard/isotop/blob/main/src/shaders.wgsl) |
| `Layout::spot`, `pick_radius`, `Pointer::hover_over`, click handling in `pointer_event`, `thing_popup` | [`src/app/screen.rs`](https://github.com/tfpickard/isotop/blob/main/src/app/screen.rs), [`src/app/pointer.rs`](https://github.com/tfpickard/isotop/blob/main/src/app/pointer.rs), [`src/app/overlay.rs`](https://github.com/tfpickard/isotop/blob/main/src/app/overlay.rs) |
| The pick rules for view authors | [`docs/VIEW_CONVENTIONS.md`](https://github.com/tfpickard/isotop/blob/main/docs/VIEW_CONVENTIONS.md), section "Picking" |
| The `ValidPicks` property | [`src/harness.rs`](https://github.com/tfpickard/isotop/blob/main/src/harness.rs) |

## References

* P. M. Fitts, "The information capacity of the human motor system in controlling the amplitude of movement", *Journal of Experimental Psychology* 47(6), 1954, 381-391.
* T. Akenine-Moller, E. Haines and N. Hoffman, *Real-Time Rendering*, 4th ed., CRC Press, 2018 (the depth buffer; picking and selection).
* *XTerm Control Sequences* (the mouse tracking modes 1000, 1002, 1003, 1006 and the pixel mode 1016 used for cell and pixel reports).
