# Theory of operation

One page per view that rests on real science or mathematics: what the picture claims, where the idea came from, the background you need to read it, how isotop implements it and where it departs from the real thing. Start with [VIEW_CONVENTIONS.md](../VIEW_CONVENTIONS.md) for the rules every view follows and the README for usage.

| Page | Idea |
|---|---|
| [Horizon](horizon.md) | Schwarzschild geodesics, gravitational lensing, Page-Thorne disks, the OOM killer |
| [Cortex](cortex.md) | Izhikevich spiking neurons, synaptic delay, Golomb synchrony |
| [Ptolemy](ptolemy.md) | Epicycles as a discrete Fourier series, retrograde motion |
| [Tesseract](tesseract.md) | 4D projection and rotation, Lorentz boosts, Cherenkov cones |
| [Coop](coop.md) | The Vicsek flocking model, cgroups, the pecking order |
| [Matrix](matrix.md) | The system journal, syslog severities, bitmap fonts, digital rain |
| [Necropolis](necropolis.md) | How processes die, zombies, pid reuse, a history of necropolises |
| [Arbor](arbor.md) | Leonardo's branching rule, phyllotaxis, damped oscillators, a history of trees |

## Page pattern (draft)

Each page has these sections, in this order. Pages run about 100 to 140 lines.

1. **What you are looking at.** A table from each visual element to the quantity it measures. Anything decorative is said so.
2. **History.** Always present. Where the idea came from and who found it, and for views whose subject is a thing (necropolises, trees, chickens) the history of the thing itself. Prefer specific, checkable facts (names, dates, places) to general ones. Where sources disagree, or a story rests on one person's recollection, say so in the text.
3. **Background theory.** The physics or mathematics, with the equations that matter and no more. Derive only what the reader cannot look up in a minute, and give a few worked numbers from the constants the view uses.
4. **How isotop implements it.** The algorithm, constants and file names. The *why* of every non-obvious choice.
5. **Faithful, stylized, invented.** An honest ledger. It is the realness contract of the conventions document applied to the theory.
6. **Reading the picture.** What a pattern on screen means about the machine.
7. **Further reading.** Primary sources first.

Conventions:

- Equations are LaTeX in `$...$` and `$$...$$`, which GitHub renders.
- Every number quoted from the code is a constant that exists today. When a constant changes, the page changes in the same PR.
- A claim about the code names the function or constant, so the reader can check it. Check each one against `src/` before writing it; review has caught several that were remembered, not read.
- Where a prototype in `docs/mockups/code/` is wrong, the page says so.
- Markdown files count in `docs/METRICS.md`: run `python3 scripts/code_report.py` after adding or editing a page.
