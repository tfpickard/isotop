# isotop

A living picture of your machine inside the terminal: real process data, a 3D scene, pixel graphics through
the Kitty graphics protocol, and the physics, mathematics and biology that make each picture worth looking at.

isotop has no practical use. It is maximalism for its own sake, and this wiki is its academic companion:
the **theory of operation**, the **mathematics** behind every view, the **history** of the ideas, **media**,
and, as it grows, **worked examples and recipes**. Start with the [Philosophy](Philosophy) if you want to
know why, or the [Theory Overview](Theory-Overview) if you want to know how.

![All fourteen views](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/contact-sheet.webp)

## Where to start

| If you want | Read |
|---|---|
| The idea and the one rule it obeys | [Philosophy](Philosophy) |
| The whole machine on one page | [Theory Overview](Theory-Overview) |
| To see everything | [Gallery](Gallery) |
| A view you have seen | pick one from the list below |
| A view that does not exist yet | the [Planned](#planned-views) list |
| To add a view | [Guide: Writing a View](Guide-Writing-a-View) |
| Commands that work | [Guide: Recipes](Guide-Recipes) |
| A word you do not know | [Glossary](Glossary) |

## Theory of operation

Data in, pixels out, with a contract at every seam.

1. **Sampling** &mdash; [Linux](Theory-Sampling-Linux), [macOS](Theory-Sampling-macOS),
   [the background sampler and demand](Theory-Background-Sampler-and-Demand).
2. **The model** &mdash; [rates, smoothing and unknowns](Theory-Model-and-Rates),
   [life events](Theory-Life-Events), [history](Theory-History), [the journal](Theory-Journal).
3. **The scene** &mdash; [display list](Theory-Scene-and-Display-List),
   [camera and projection](Theory-Camera-and-Projection), [stable layout](Theory-Stable-Layout),
   [colour](Theory-Colour), [simulation toolkit](Theory-Simulation-Toolkit).
4. **Pixels** &mdash; [CPU rasterization](Theory-Rasterization),
   [the GPU and CPU/GPU parity](Theory-GPU-and-Parity), [picking](Theory-Picking),
   [the terminal graphics protocol](Theory-Terminal-Graphics).
5. **Guarantees** &mdash; [determinism](Theory-Determinism), [text safety](Theory-Text-Safety).

## Mathematics

[Fourier epicycles](Math-Fourier-Epicycles) &middot; [the wave equation](Math-Wave-Equation) &middot;
[flocking](Math-Flocking) &middot; [spatial hashing](Math-Spatial-Hashing) &middot;
[stellar photometry](Math-Stellar-Photometry) &middot; [Lorentz boosts and Schlegel projection](Math-Lorentz-and-Schlegel) &middot;
[reaction-diffusion](Math-Reaction-Diffusion) &middot; [spiking neurons](Math-Spiking-Neurons) &middot;
[the Schwarzschild solution](Math-Schwarzschild)

## Views

Fourteen exist today.

| | | |
|---|---|---|
| [City](View-City) | [Orbit](View-Orbit) | [Ripple](View-Ripple) |
| [Flow](View-Flow) | [Cores](View-Cores) | [Cells](View-Cells) |
| [Strata](View-Strata) | [Globe](View-Globe) | [Reef](View-Reef) |
| [Coop](View-Coop) | [Matrix](View-Matrix) | [Ptolemy](View-Ptolemy) |
| [Tesseract](View-Tesseract) | [Necropolis](View-Necropolis) | |

### Planned views

Fifteen more are specified, tracked on the project board, and documented here theory first. Each page says
plainly that it describes a design.

| | | |
|---|---|---|
| [Mold](Planned-Mold) | [Chamber](Planned-Chamber) | [Horizon](Planned-Horizon) |
| [HR](Planned-HR) | [Hive](Planned-Hive) | [Anthill](Planned-Anthill) |
| [Switchboard](Planned-Switchboard) | [Plumbing](Planned-Plumbing) | [Morph](Planned-Morph) |
| [Murmuration](Planned-Murmuration) | [Cortex](Planned-Cortex) | [Ouija](Planned-Ouija) |
| [Arbor](Planned-Arbor) | [Canyon](Planned-Canyon) | [Die](Planned-Die) |

## Guides

[Writing a view](Guide-Writing-a-View) &middot; [Recipes](Guide-Recipes) &middot;
[Headless and reproducible renders](Guide-Headless-and-Reproducible-Renders) &middot;
[Contributing to the wiki](Guide-Contributing-to-the-Wiki)

## Links

* Source: <https://github.com/tfpickard/isotop>
* Roadmap: [docs/ROADMAP.md](https://github.com/tfpickard/isotop/blob/main/docs/ROADMAP.md) and the
  [view conventions](https://github.com/tfpickard/isotop/blob/main/docs/VIEW_CONVENTIONS.md)
* Source brief for the planned views:
  [docs/briefs/views-2026-10.md](https://github.com/tfpickard/isotop/blob/main/docs/briefs/views-2026-10.md)

This wiki is generated from the `wiki/` directory of the main repository and republished on every merge to
`main`. Edit the files there, not the wiki UI.
