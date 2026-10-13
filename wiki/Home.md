# isotop

A living picture of your machine inside the terminal: real process data, a 3D scene, pixel graphics through
the Kitty graphics protocol, and the physics, mathematics and biology that make each picture worth looking at.

isotop has no practical use. It is maximalism for its own sake, and this wiki is its academic companion:
the **theory of operation**, the **mathematics** behind every view, the **history** of the ideas, **media**,
and, as it grows, **worked examples and recipes**. Start with the [Philosophy](Philosophy) if you want to
know why, or the [Theory Overview](Theory-Overview) if you want to know how.

![All eighteen views](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/contact-sheet.webp)

## History

isotop is young. The repository's first code commit is dated 7 October 2026, 23:05 in the author's time zone,
and it already contained four views (City, Orbit, Ripple and Flow), a CPU rasterizer, a wgpu rasterizer, and a
terminal layer that speaks the Kitty graphics protocol. The rest arrived in this order:

| When (author's local time) | What |
|---|---|
| 8 October 2026, 00:57 | Cores, Cells, Strata, Globe and Reef, in one commit, one hour and fifty-two minutes after the first |
| 8 October, 15:03 | Matrix |
| 9 October, 08:34 | Coop |
| 9 October, 09:42 | The macOS port |
| 11 October, 09:42 | The view registry and the view harness: from here a view is one module and one row |
| 11 October, 18:17 | Ptolemy, Tesseract and Necropolis |
| 12 October, 00:02 | Cortex, Horizon and Murmuration |
| 12 October, 11:03 | Arbor, the eighteenth view, four days and twelve hours after the first commit |

On its first day the repository received seven commits. This wiki has more than ten times as many pages.

The dates are those of the commits that first added each view's file (`git log --diff-filter=A`). The history of the
ideas behind the views is on each view's page, and the history of the project's central question is on the
[Philosophy](Philosophy) page.

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

Eighteen exist today.

| | | |
|---|---|---|
| [City](View-City) | [Orbit](View-Orbit) | [Ripple](View-Ripple) |
| [Flow](View-Flow) | [Cores](View-Cores) | [Cells](View-Cells) |
| [Strata](View-Strata) | [Globe](View-Globe) | [Reef](View-Reef) |
| [Coop](View-Coop) | [Matrix](View-Matrix) | [Ptolemy](View-Ptolemy) |
| [Tesseract](View-Tesseract) | [Necropolis](View-Necropolis) | [Cortex](View-Cortex) |
| [Horizon](View-Horizon) | [Murmuration](View-Murmuration) | [Arbor](View-Arbor) |

### Planned views

Eleven more are specified, tracked on the project board, and documented here theory first. Each page says
plainly that it describes a design.

| | | |
|---|---|---|
| [Mold](Planned-Mold) | [Chamber](Planned-Chamber) | [HR](Planned-HR) |
| [Hive](Planned-Hive) | [Anthill](Planned-Anthill) | [Switchboard](Planned-Switchboard) |
| [Plumbing](Planned-Plumbing) | [Morph](Planned-Morph) | [Ouija](Planned-Ouija) |
| [Canyon](Planned-Canyon) | [Die](Planned-Die) | |

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

The repository also keeps a shorter, in-repo set of theory pages for eight of the views in
[docs/theory](https://github.com/tfpickard/isotop/tree/main/docs/theory), with diagrams of their mechanisms.

This wiki is generated from the `wiki/` directory of the main repository and republished on every merge to
`main`. Edit the files there, not the wiki UI.
