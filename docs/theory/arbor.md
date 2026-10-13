# Arbor: the process tree as a tree

**View:** `arbor` · **Code:** `src/arbor.rs` · **Related:** the Leonardo-branching and phyllotaxis sections of `docs/VIEW_CONVENTIONS.md`

## What you are looking at

| On screen | Measured quantity |
|---|---|
| The trunk | PID 1, else the root of the largest subtree (a pale trunk is launchd) |
| A branch | one process, attached to its parent's branch |
| Branch thickness | the memory of the whole subtree it carries, via Leonardo's rule |
| Where a branch leaves its parent | a phyllotaxis slot, fixed by the order in which the parent first saw its children |
| Leaves | threads, at most 32 per twig and 6,000 in all; green when idle, red when busy |
| A falling leaf | an exited thread |
| Blossoms | a process that started less than 5 s ago |
| Roots under a soil slab | kernel threads |
| The sway | CPU pressure, as a wind |
| A pale branch | a parent isotop cannot read (run with sudo) |

Branch length, elevation and azimuth are *structural*, not measurements: they come from depth and slot, so layout never depends on CPU, memory or population.

## History

**Trees are not a family; they are a habit.** A tree is a growth form, a plant that stands on its own wood, and plants have invented it several separate times. The first plants to grow tall were small: *Cooksonia* and its relatives, a few centimetres of bare forked stem, around 425 million years ago. By the middle Devonian some stood taller than a person, and the biggest organism on land was not a plant at all but *Prototaxites*, a fungus that grew to several metres.

**Wattieza** is the oldest tree known in the fossil record. Its stumps, found in the Gilboa quarry in New York State, are about 385 million years old: tree-fern-like cladoxylopsids with a hollow trunk that put almost all their energy into growing upward, a crown of frond-like branches, and a shallow root system that never spread. Reports differ on how much older it is than the next tree, by roughly 15 to 25 million years. Nothing else on the planet was nearly as tall.

**Archaeopteris** followed, and it is the ancestor of what we mean by "tree". It grew true wood by secondary growth, with a ring of new wood laid down each year, branched in all directions, carried flat leaves, and sent roots deep into the ground. It formed forests of trees up to some tens of metres tall by the end of the Devonian. Its roots broke rock and drew down carbon dioxide, and one hypothesis is that this helped cool the climate and altered ocean chemistry in the Late Devonian. That is still a hypothesis.

**The coal forests** of the Carboniferous (about 360 to 300 million years ago) were different again: giant club-mosses (*Lepidodendron*, *Sigillaria*, tens of metres tall), horsetail trees (*Calamites*) and early seed plants in swamps. Trees died and sank into the swamp faster than they decayed, and the carbon they fixed became coal, which is why our industrial age runs on Carboniferous trees. One popular explanation, that the forests could pile up carbon because nothing yet could rot lignin, was proposed in 2012 and has since been disputed. It is a good example of a story that is neat enough to be suspicious.

Conifers became common in the Permian and Triassic, and flowering plants (angiosperms) spread from the Cretaceous, about 130 million years ago, and now make most of the world's trees.

**Humans and trees.** Dendrochronology, dating by ring patterns, was founded by the American astronomer A. E. Douglass early in the 20th century. The oldest individual trees are bristlecone pines of nearly five thousand years; *Pando*, a quaking aspen grove in Utah, is one clonal organism. In 1994 a stand of *Wollemia*, a conifer known only from fossils, turned up alive in a gorge outside Sydney.

**Branching rules.** Leonardo da Vinci's notebooks state that when a tree divides, the thickness of the daughters, taken together, equals the thickness of the parent. In 1964 Shinozaki and colleagues proposed the pipe model: a tree is a bundle of unit-thickness pipes, one per leaf-bearing twig, so a branch's cross-section tracks the leaf area it carries. Christophe Eloy (2011) showed that the same rule follows from a different principle, equal risk of snapping in the wind everywhere. And the spiral arrangement of leaves and branches, **phyllotaxis**, was a puzzle for centuries. Karl Schimper and Alexander Braun found the Fibonacci regularities in the 1830s, the Bravais brothers measured the divergence angle near 137.5° in 1837, and in 1992 Stéphane Douady and Yves Couder reproduced the pattern with magnetized droplets, showing that it emerges from each new bud being placed where the previous ones leave the most room.

**The tree as an idea.** Darwin sketched a branching tree in a notebook in 1837, beside the words "I think". Haeckel drew oaks of life in 1866. The process tree is the same figure: Unix has grown processes by `fork` since its early days, so every process descends from PID 1, and the only difference from the oak is that a process can die before its children (they are adopted by PID 1, a sort of grafting).

## Background theory

### Leonardo's rule

Let a parent branch have cross-sectional area $A_p$ and daughters $A_1, \dots, A_n$. Leonardo's rule is

$$A_p = \sum_i A_i, \qquad\text{or in radii}\qquad r_p^2 = \sum_i r_i^2$$

so a branch that splits in two equal halves has daughters of radius $r_p/\sqrt{2} \approx 0.707\,r_p$. Compare Murray's law for blood vessels, $r_p^3 = \sum r_i^3$, which minimizes the work of pumping a viscous fluid; the cube is the signature of laminar flow. A tree does not pump, so it follows the square, which is the signature of area conservation: sap conducting tissue in parallel (the pipe model), or equal stress under wind (Eloy). Real trees follow the rule roughly, and many species do not.

### Phyllotaxis

If each new organ is placed at an angle $\alpha$ from the last, then $\alpha$ a rational fraction of a turn makes the organs line up in spokes and waste room. The most irrational angle, the one worst approximated by fractions, is the golden angle

$$\alpha = 2\pi\left(1 - \frac{1}{\varphi}\right) \approx 137.508^\circ, \qquad \varphi = \frac{1+\sqrt5}{2}$$

whose continued fraction is all ones, so no rational approximation is good for long. Successive organs at this angle spread into the Fibonacci spirals seen in sunflower heads and pine cones.

### Damped oscillators and wind

Each branch is a damped harmonic oscillator pulled towards the lean the gust asks for:

$$\ddot{x} = \omega^2\,(x_{\text{target}} - x) - 2\zeta\omega\,\dot{x}$$

Here $\omega$ is the branch's natural frequency and $\zeta = 0.12$ its damping ratio. That is *underdamped* ($\zeta < 1$): a pulse overshoots by $e^{-\pi\zeta/\sqrt{1-\zeta^2}} \approx 68\%$ of the step and rings, with an amplitude envelope that shrinks as $e^{-\zeta\omega t}$. At the top frequency (3 Hz, $\omega \approx 18.8$ rad/s) that is a time constant of $1/(\zeta\omega) \approx 0.44$ s. Thin twigs have high $\omega$ and shake fast; heavy limbs have low $\omega$ and roll slowly. That is what real trees do and why a canopy looks alive.

The integration is semi-implicit Euler at a fixed step of 0.05 s, which is stable while $\omega\,\Delta t < 2$; at 3 Hz the product is 0.94.

## How isotop implements it

1. **The tree.** `Tree` builds nodes from parent links. A parent hands out its phyllotaxis slots in the order it first sees its children, from a counter that only rises, and children it sees at once are ordered by start time then pid. A branch therefore keeps its slot when other children come and go. The azimuth of slot $k$ advances by the golden angle; `SLOT_ANGLE` is 137.5°.
2. **Geometry** is a pure function of depth and slot: a branch of depth $d$ is $14\times0.84^d$ units long (11.8 at depth 1, 8.3 at depth 3); the trunk's children leave at 66° from its axis and deeper branches at 52°.
3. **Thickness.** Each node's *own* memory and its *subtree's* memory are computed. A branch's radius is $R_{\text{trunk}}\sqrt{A/A_{\text{trunk}}}$ with $R_{\text{trunk}} = 1.6$, never under a floor of 0.05. Along the branch the area steps down at each junction: `levels` takes the subtree area at the base, subtracts each child's subtree as its junction is passed, and ends at the branch's own memory at the tip. So Leonardo's rule holds *exactly at every junction*, and a branch with a child of a quarter of its area tapers to $\sqrt{3/4}\approx 0.87$ of its radius there. A short ramp (5% of the branch) blends each step, and children whose junctions are within 2% share one step, so a parent of hundreds of children is still a handful of tube pieces.
4. **Roots** hang from the trunk under an opaque soil slab. Kernel threads are the roots, and since they number in the dozens or hundreds they are scaled by the same area law against `ROOT_COLLAR` (0.55 of the trunk radius).
5. **Leaves** are threads. The count per twig is capped (32) and the whole scene is capped (6,000); a bucket of threads shares a leaf. A falling leaf takes 4 s to land, rests 30 s on the soil and is gone; at most 400 are falling or resting.
6. **Wind** is CPU pressure, eased towards its reading with a 1.5 s time constant. Each branch has an oscillator driven by a gust made of a steady push plus two incommensurate sines, so no two branches move alike and the motion does not repeat.

## Faithful, stylized, invented

- **Faithful:** the parent-child structure, subtree memory as branch area, Leonardo's rule at each junction, the golden-angle slotting, the damped oscillator, kernel threads as roots.
- **Stylized:** the lengths ($0.84^d$) and elevations are chosen to read well; thread counts are capped; leaves are idle-green or busy-red by CPU rather than any physiology.
- **Invented:** wind as CPU pressure. A real wind pushes on real surfaces. Here it is a mapping, and the only reason a tree is the right body for it is that something has to make the canopy move so the eye can separate branches.
- **Not modelled:** growth. A process tree does not grow like a tree: a branch appears and is thick at once.

## Reading the picture

- A trunk with one enormously thick branch is a machine dominated by one subtree (a browser, a database).
- A fat branch with a thin one off it is a parent holding most of its subtree's memory itself.
- A crown of many thin twigs is a fork-heavy workload: build, test, a shell pipeline.
- A pale branch is a missing parent. Re-run with enough privilege to read it.
- Roots that thicken are kernel work (workqueues, I/O threads) accumulating.
- A canopy that thrashes means CPU pressure; still air means the machine has headroom, or that the pressure source is unreadable, which the legend says.

## Further reading

- Stein, W. E. et al. (2007), "Giant cladoxylopsid trees resolve the enigma of the Earth's earliest forest stumps at Gilboa", *Nature* 446, 904.
- Meyer-Berthaud, B., Scheckler, S. E. & Wendt, J. (1999), "*Archaeopteris* is the earliest known modern tree", *Nature* 398, 700.
- Eloy, C. (2011), "Leonardo's rule, self-similarity and wind-induced stresses in trees", *Phys. Rev. Lett.* 107, 258101.
- Shinozaki, K. et al. (1964), "A quantitative analysis of plant form: the pipe model theory", *Japanese Journal of Ecology* 14.
- Douady, S. & Couder, Y. (1992), "Phyllotaxis as a physical self-organized growth process", *Phys. Rev. Lett.* 68, 2098.
- Prusinkiewicz & Lindenmayer, *The Algorithmic Beauty of Plants* (1990).
- Floudas, D. et al. (2012), *Science* 336, 1715, and Nelsen, M. P. et al. (2016), *PNAS* 113, 2442, for the lignin and coal debate.
