# docs/figures

## OVERVIEW
About 90 deterministic stdlib-only Python scripts, each writing one or more SVGs to `docs/media/figures/`. Own file
because the figure rules (kits, determinism, slugs) are strict and differ from the rest of `docs/`.

## WHERE TO LOOK
| Task | File | Symbol / notes |
|---|---|---|
| Generic SVG helpers | `figkit_w4.py`, `figkit_w3.py` | stdlib kits |
| Plot primitives | `svgkit_w6.py`, `svgkit_w8.py` | stdlib kits |
| Sampling and planned-view plots | `sampling_plot.py`, `svgplot_planned.py`, `svgplot_views_cores_reef.py` | stdlib kits |
| Model modules shared by scripts | `hh_w8.py`, `izh_w8.py`, `rdsim_w8.py`, `murm_w8.py`, `flock_sim_w6.py`, `epicycle_w6.py` | simulations the plots call |
| A page's figure | `<page>-<slug>.py` | e.g. `strata-window.py`, `fourier-epicycle-chain.py`, `model-smoothing-step.py` |
| Matplotlib theory diagrams | `../theory/figures/make_figures.py` | not here; the only place matplotlib is allowed |
| Output | `../media/figures/<slug>.svg` | written relative to the repo root |

## CONVENTIONS
- Run from the repo root: `python3 docs/figures/strata-window.py`.
- Standard library only. Deterministic: no clock, environment or unseeded randomness; two runs write identical bytes.
- Slug prefix is the page group (`strata-`, `fourier-`, `model-`, `mold-`, `theory-`, ...); list `docs/media/figures/` first.
- The SVG carries its own `#0e1116` background rect, light strokes and `#c9d1d9` text, a `viewBox`, a `<title>`,
  axis labels with units and `font-family="system-ui, sans-serif"`.
- Plot the page's mathematics, two to five figures per page; take constants from the named `src/` file.
- Reuse the kit nearest the topic. Convert to PNG with `convert` and look at it before committing.
- Commit the script and its SVG together.

## ANTI-PATTERNS
- Do not add a fifth kit; extend or reuse `figkit_w4.py`, `svgkit_w6.py`, `svgkit_w8.py`, `sampling_plot.py` or `svgplot_planned.py`.
- Do not import matplotlib, numpy or any third-party package here.
- Do not hand-draw an SVG or commit a screenshot of a plot.
- Do not let a plotted constant disagree with the Rust source.
- Do not decorate: a figure that plots no equation of the page does not belong.

## COMMANDS
- `python3 docs/figures/<script>.py` then `cmp` against a second run to prove determinism.
- `convert docs/media/figures/<slug>.svg /tmp/<slug>.png`
