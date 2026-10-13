# Philosophy

isotop has no practical use. A process list that did the job would be `top`, which exists, is faster, and
fits in a corner of the screen. isotop is instead a machine for turning the hum of an operating system into
pictures that are worth looking at for their own sake, and for taking the physics, mathematics and biology
behind those pictures seriously enough to write them down. This wiki is where that writing lives.

It is maximalist on purpose. There are eighteen views and eleven more specified, a CPU rasterizer and a GPU
rasterizer that must agree pixel for pixel, a Fourier transform written in-tree so a planet can carry an
epicycle chain, a Lorentz boost applied to the history of a shell. None of that is proportionate to the problem
of "which process is using the CPU". It is proportionate to a different problem: *how much of the structure
of a running computer can be made visible, and how beautiful can the visible thing be without lying?*

## History

The question of what a machine is doing has been asked, with software, for as long as there have been machines
to ask it of. The tools that answer it each reflect their decade.

* **`/proc`.** Tom Killian built the first process filesystem for Eighth Edition Unix at Bell Labs and described
  it in "Processes as Files" at the USENIX conference in the summer of 1984. Its purpose was practical: to replace
  the `ptrace` system call with files that could be read and written. Linux's `/proc`, which isotop reads, descends
  from the idea, not the code.
* **`top`.** The first version was written by William LeFebvre and posted to the `net.sources` newsgroup in 1984.
  Where it began is disputed: later copyright notices in the code reach back to 1980 and credit an earlier author at
  Lucasfilm. Either way, a screen of the busiest processes, refreshed in place, has been the default answer for
  four decades.
* **Treemaps.** Ben Shneiderman and colleagues at the University of Maryland devised the treemap in the early 1990s
  (the first papers are dated 1991) to answer a question about disk space: where on this disk did the space go? It
  is the first widely used picture in which area is a measurement of a resource, and the ancestor of every
  disk-usage visualizer since.
* **Task Manager.** The Windows NT Task Manager, which shipped with NT 4.0 in 1996, was written by Dave Plummer at
  home, as a shareware idea he meant to sell. Dave Cutler, the architect of NT, saw it and agreed to take it into
  the product, and Plummer donated it.
* **`htop`.** Hisham Muhammad released `htop` in May 2004, as an interactive alternative to `top` that shows every
  process, with colour and a mouse.

Each of these was built to be *used*. A picture of a process table is a different kind of artifact, and the
work isotop belongs to is the one that treats the data as something worth looking at: software cities and
treemaps, as visualization research, and a long tradition of screensavers and demoscene pieces, as art. isotop
is in the second tradition with the discipline of the first. The next section says what the discipline is.

## The one rule: the realness contract

The art is allowed to be extravagant. It is not allowed to be fake. Every visual element in a view maps to a
measured quantity, or the documentation declares it **decorative**. Each view's page opens its mapping with a
table: visual element, measured source, transform (log, square root, smoothing constant, quantum). When a source
cannot be read (permissions, kernel version, hardware), the view degrades visibly and says so in its status
strip. Live mode never fills a gap with invented numbers. Simulations advance on wall-clock time, not frames,
and their randomness is seeded from process identity so the same machine state draws the same picture.

This is the constraint that makes the project interesting rather than merely pretty. A picture of a bubble
chamber is just a picture. A picture of a bubble chamber in which every track's curvature is a measured
quantity, and in which you can say exactly which, is a small instrument. The question each page asks is the one
a physicist asks of any plot: *what does this let me know, and what does it not?*

## What you can and cannot know

Every view is a lossy channel. A process table sampled once a second cannot see anything faster than a half
second; a kernel counter is cumulative, so a rate is a difference of two readings and the first reading has no
rate; a value that was not measured is *unknown*, not zero. Each page therefore has a section on reading the
picture: which things are measured, which are derived, which are decorative, and what is invisible. The
pages on [sampling](Theory-Sampling-Linux), [the model](Theory-Model-and-Rates) and
[determinism](Theory-Determinism) are the formal version of the same stance.

## Why physics, why biology, why history

The metaphors are not arbitrary. A process is born by fork, lives by competing for a finite resource, and dies
leaving a record; so does a particle in a bubble chamber ([Planned Chamber](Planned-Chamber)), a hypha in a
slime mold ([Planned Mold](Planned-Mold)), a hen in a coop ([View Coop](View-Coop)). Memory pressure that ends
in the kernel killing something is, at the level of structure, a horizon ([View Horizon](View-Horizon)).
The history of astronomy gave us a way of describing any periodic motion as a chain of circles
([View Ptolemy](View-Ptolemy)) long before Fourier made the idea exact. Where the correspondence is deep, the
page says why. Where it is only a pleasing resemblance, the page says that too.

## How the wiki is organised

* **Theory** pages describe how isotop works, from the kernel counters to the pixels in the terminal.
  Start with the [Theory Overview](Theory-Overview).
* **Math** pages treat a technique on its own terms, independent of any one view: the
  [wave equation](Math-Wave-Equation), [Fourier epicycles](Math-Fourier-Epicycles),
  [flocking](Math-Flocking), [reaction-diffusion](Math-Reaction-Diffusion), and so on.
* **View** pages cover each implemented view; **Planned** pages cover the views that are specified but
  not yet built, theory first, clearly marked as designs.
* **Guides** hold the practical material: [writing a view](Guide-Writing-a-View) (there is no dynamic plugin
  system, and the page says so), [recipes](Guide-Recipes), and how to
  [render reproducibly](Guide-Headless-and-Reproducible-Renders).
* The [Gallery](Gallery) is the media; the [Glossary](Glossary) is the vocabulary.

## Non-goals

isotop does not try to be fast at the expense of being legible, minimal at the expense of being interesting, or
useful at the expense of being true. It never sends anything off the machine. It never escalates privilege. It
does not pretend a decorative element is a measurement. Beyond that, the design goal is to see how far a
terminal, a handful of kernel interfaces and some mathematics can be pushed, and to keep an honest account of
what was done and why.
