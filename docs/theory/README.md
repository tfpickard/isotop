# Theory of operation

One page per view that rests on real science or mathematics: what the picture claims, the background you need to read it, how isotop implements it and where it departs from the real thing. Start with [VIEW_CONVENTIONS.md](../VIEW_CONVENTIONS.md) for the rules every view follows and the README for usage.

| Page | Idea |
|---|---|
| [Horizon](horizon.md) | Schwarzschild geodesics, gravitational lensing, Page-Thorne disks, the OOM killer |
| [Cortex](cortex.md) | Izhikevich spiking neurons, synaptic delay, Golomb synchrony |
| [Ptolemy](ptolemy.md) | Epicycles as a discrete Fourier series, retrograde motion |
| [Tesseract](tesseract.md) | 4D projection and rotation, Lorentz boosts, Cherenkov cones |

## Page pattern (draft)

Each page has these sections, in this order:

1. **What you are looking at.** A table from each visual element to the quantity it measures. Anything decorative is said so.
2. **Background theory.** The physics or mathematics, with the equations that matter and no more. Derive only what the reader cannot look up in a minute; cite the rest.
3. **How isotop implements it.** The algorithm, constants and file names. The *why* of every non-obvious choice.
4. **Faithful, stylized, invented.** An honest ledger. It is the realness contract of the conventions document applied to the theory.
5. **Reading the picture.** What a pattern on screen means about the machine.
6. **Further reading.** Primary sources first.

Conventions:

- Equations are LaTeX in `$...$` and `$$...$$`, which GitHub renders.
- Every number quoted from the code is a constant that exists today. When a constant changes, the page changes in the same PR.
- A claim about the code names the function or constant, so the reader can check it.
- Where a prototype in `docs/mockups/code/` is wrong, the page says so.
