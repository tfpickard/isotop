# View Ptolemy

isotop becomes the Earth at the centre of a geocentric cosmos. The seven processes whose CPU and memory varied most
over the last minute are the planets, each carried on a deferent circle and a chain of twelve epicycles that is
the discrete Fourier transform of that minute, replaying it once a minute; every other process is a fixed star on an
outer ring, as bright as its memory. Where a planet's apparent longitude seen from Earth runs backwards, its trail turns
red and the status line says so. It is the most mathematical of the fourteen views: the picture is, to rounding, a
theorem about Fourier series dressed as second-century astronomy.

![Ptolemy: Earth at the centre, planets on deferents with epicycle chains and red retrograde trails, fixed stars on the outer ring](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/renders/ptolemy.webp)

![The README image of the same view (demo workload, 1600 by 900)](https://raw.githubusercontent.com/tfpickard/isotop/main/docs/media/ptolemy.webp)

## The system being modelled

Ptolemaic astronomy (the *Almagest*, about 150 CE) predicts the positions of the Sun, Moon and five naked-eye planets as
seen from a fixed Earth. A planet rides an *epicycle*, a small circle whose centre travels on a larger circle, the
*deferent*; seen from Earth the planet slows, stops, runs backwards against the stars (*retrograde motion*), stops
again and resumes. The geometry is derived on [Math Fourier Epicycles](Math-Fourier-Epicycles): retrograde motion exists
exactly when the epicycle's tip speed exceeds the deferent's speed. Ptolemy's refinements (eccentric deferents and
the *equant*, a point about which the epicycle's centre turns uniformly) make the model predictive; isotop uses none of them. Its
spheres are the traditional order of the seven wanderers, Moon, Mercury, Venus, Sun, Mars, Jupiter, Saturn, from the
centre outwards, and the outermost sphere carries the fixed stars.

Why it is a good metaphor for process data: a process's resource use is a closed-ish trajectory in a plane (CPU against
memory) with a slow trend and several fast cycles. A sum of rotating vectors is the natural way to describe cycles;
a planet is a process, and a process in retrograde is one whose short-period swings outrun its long-period drift.

## Theory

The mathematics (phasors, the DFT, Parseval's optimal truncation, Gibbs ringing, the sliding-window correction and the
retrograde criterion $\operatorname{Im}(\bar p\,p') < 0$) is on [Math Fourier Epicycles](Math-Fourier-Epicycles). What this view adds is
the *composition*: the apparent position of a planet is

$$
p(t) = R_s\, e^{i(\phi + 2\pi t/T_s)} + \sum_{j=1}^{12} g\, c_j\, e^{2\pi i f_j t/60\,\mathrm{s}},
$$

a uniform deferent of radius $R_s$ and period $T_s = 1200\,\mathrm{s}\,(R_s/R_{\mathrm{Saturn}})^{3/2}$ (Kepler's third law applied to made-up radii),
plus the twelve largest DFT terms $c_j$ of the process's last 60 samples, scaled by one global gain $g$. Because the chain's
terms turn once to thirty times a minute and the deferent turns once in 3.7 to 20 minutes, an epicycle only 0.3 to 0.5 units wide outruns the
deferent: retrograde is the common state, not a rarity. That is a difference from the real sky and the reason the red
trails look like saw-teeth rather than the classical loops (see below).

## Mapping

| Visual element | Measured quantity (source) | Transform |
|---|---|---|
| Earth, centre | isotop's own process (`Snapshot::observer`; in `--demo` a synthetic pid 999) | Labelled `isotop (pid N)` and `X cores \| Y MiB` (CPU / 100, memory in MiB); clickable when isotop is among the drawn processes |
| Planet | The seven drawn processes with the highest variance score over their last 60 samples of the shared history (`history.rs`) | Score = variance of CPU in cores + variance of memory in doublings (log2 MiB, floored at 1 MiB). Needs at least 30 samples and a score above 0; isotop itself is excluded. An incumbent keeps its sphere while it ranks tenth or better |
| Sphere (which circle) | Mean CPU over the window | When the membership changes the seven are sorted busiest innermost: Moon, Mercury, Venus, Sun, Mars, Jupiter, Saturn at 0.245, 0.33, 0.415, 0.5, 0.585, 0.67, 0.755 of the sky radius (40 units); a planet that changes sphere eases there over 1.5 s |
| Deferent period | Sphere radius | 1200 s x (r / r_Saturn)^1.5; starting angle from the identity hash (decorative) |
| Epicycle chain | CPU in cores + i x memory in doublings, mean removed, 60 samples | Direct 60-point DFT, the 12 largest terms without the mean, largest first from the deferent point; the tip is the planet. Samples before a process's first are held at its first reading |
| Chain length | Sum of coefficient magnitudes | One global gain, soft-limited so the radii sum to $7.6\tanh(3.4\sum\lvert c\rvert/7.6)$ units |
| Planet size | Memory | 0.34 + 0.105 log2(max(memory / 64 MiB, 1)) units, at most 1.2 |
| Planet colour | Kind and state | `palette::kind_color`, as in the other views |
| Trail | The chain and deferent as they stand now | Positions over the last min(240 s, 0.8 x deferent period) on a 0.25 s grid; the oldest 45 % fades |
| Red trail, red glow, dashed sight line, marker on the ecliptic scale | Retrograde: $\operatorname{Im}(\bar p\,p') < 0$ | Exact analytic derivative of the deferent plus the chain; sight line from Earth to the ecliptic scale |
| Fixed star | Every other drawn process | Angle and radius (0.935 + 0.06 u) x 40 from the identity hash, on a ring that turns once in 60 s |
| Star brightness | Memory (RSS; the physical footprint on macOS) | $m = -2.5\log_{10}(\mathrm{memory}/1\,\mathrm{GiB})$, floored at 1 MiB; core and halo from the shared point spread ([Math Stellar Photometry](Math-Stellar-Photometry)) |
| Nova | A process that appeared since the sample before (within 8 s) | Brightens 4 magnitudes in 0.3 s, then decays with e-folding time 2 s, over 6 s |
| Fade | A process that exited | Dims out over 1.5 s at its seat on the ring |
| Legend lead | Samples in the window; planets in retrograde | `the heavens are still forming: N of 30 samples`, then `X is in retrograde` / `X and Y are in retrograde` |

**Decorative:** the armillary rings (outer sphere, two ecliptic hoops, two tilted hoops), the ecliptic degree scale and its labels, the
faint spokes from Earth to each deferent point, the dot on the deferent, the colours of the epicycle circles and the chain arm,
Earth's continents, the starting angle on the deferent, the names of the spheres (they stand in for a ranking by mean CPU, not for real distances), and the turning of
the star ring.

## How isotop realises it

**Selection and hysteresis.** `Ptolemy::observe` runs once per new sample. It takes the last `SERIES = 60` samples of the
shared history at or before the shown snapshot (so pause and rewind show the past they were taken in), scores every drawn
process with `measure()` (at least `FORMING = 30` readings; the variance of CPU in cores plus the variance of memory in
doublings), and ranks candidates by score, ties by identity. Existing planets that still rank within `KEEP_RANK = 10`
keep their sphere; empty spheres are filled with the best candidates that were not planets before. Only when the
*membership* changes are the seven re-sorted by mean CPU; otherwise they stay in their spheres however the ranks shuffle.
The test `wanderers_keep_their_spheres_while_ranks_shuffle_inside_the_top_ten` fixes this.

**The chain.** `signal()` builds the complex series, `epicycles()` the twelve largest terms with the phase correction of the sliding
window, `scaled()` applies the gain. A new planet starts with its terms at their targets; an existing one `retarget`s, easing every
coefficient to the new target with a smoothstep over the measured interval between samples (clamped to 0.1 to 10 s), starting
at the instant the sample arrived (`history.arrived()`) so the easing does not depend on the frame rate. Deferents are `Deferent` values that
integrate the angle across a re-sort so it stays continuous.

**Replay and trail.** The chain replays the window once per `REPLAY = 60` s of scene time at any `--sample-ms`. The trail is *recomputed every frame*
from the current coefficients and deferent: it is where the planet would have gone with the epicycles it has now, not a recording. The code steps
`rotor`s back by one grid step per point so a trail costs one complex multiply per term per point.

**Stars, novae, fades.** `star_seat` places each non-planet process on the ring from a hashed angle and radius. Births and exits come from `History::changes` over the last
`FLASH_WINDOW = 8` s, so a new process flares as a nova (`stellar::flash(Flash::Nova, age)`) and an exited one fades (`Flash::Fade`), even in a headless frame. A process that was a planet when it exited does not fade on the ring.
(The supernova timeline in `stellar.rs`, for exits above 1 GiB, exists but this view does not use it.)

**Drawing and parity.** Strokes (trails, hoops, chains, sight lines) are drawn as pairs of triangles by `stroke()`, not as hardware lines, so the CPU and GPU rasterizers fill them alike;
depth offsets (`BEHIND_RINGS`, `BEHIND_TRAILS`, `TRAIL_BAND` per sphere, `BEHIND_CHAINS`) separate the many flat strokes that all lie on the ground plane so neither backend breaks a tie. The row has `haze: false` (trail colours are
pre-blended against the plain space sky), `flash: false` (stars flash their own novae) and its harness reference time is 300 s, because the window fills only after a minute.

**Picking and stability.** Planets and stars are pickable spheres; the harness checks the *seats* (a star's seat depends on the identity only; Earth's is the centre), not the planets, whose membership is a measurement.
The tour visits planets by variance score.

**Degradation.** Ptolemy reads only per-process CPU and memory and isotop's own row, so it has no optional source to lose. If isotop's own process is not read, Earth has no figures and is labelled `isotop`.
On macOS the picture is otherwise the same, with memory as the physical footprint.

## Reading the picture

What I see in the repository's renders. At the default `--time 12` the heavens are empty: Earth, the ring of sphere
and degree marks, the tilted armillary hoops and a ring of small coloured stars, and the status line says the heavens are
still forming. At `--time 300` (a full window) seven coloured spheres sit on seven concentric circles around the
teal Earth, each trailing a long coloured line whose red stretches make it look like a saw-blade; the colours are the
kind colours (teal, slate, purple, gold). Faint small circles hang about each planet: the epicycles. One planet has a dashed red line to Earth and a red wedge on the outer degree scale: it is in retrograde now.

* **Measured and derived.** The chain's shape is the minute of CPU and memory, to the truncation error in
  [Math Fourier Epicycles](Math-Fourier-Epicycles). The red stretches are an exact criterion on that chain. The size of a planet is memory. Which sphere a planet is on is its *rank by mean CPU*
  among the seven wanderers at the moment the membership last changed.
* **Not history.** The tip is *not* the process's current state. It is the replay of the minute (current to within the last one or two samples' easing), at a position in the replay that is a function of the clock; the trail is hypothetical. Two planets in retrograde at once are not synchronised with anything real.
* **The wrap.** Each minute the tip snaps from the last sample back to the first. For a process whose memory climbs through the whole window this is a visible jump, with Gibbs ringing beside it.
* **Handedness.** Counter-clockwise epicycles mean memory lags CPU by a quarter period, clockwise that it leads.
* **Saw-teeth.** Real planets in the Ptolemaic model loop a few times per synodic cycle because the deferent is nearly as fast as the epicycle. Here epicycle terms turn 1 to 30 times a minute and the deferents 0.05 to 0.27 times, so the epicycles are 4 to 600 times faster in turns
  and loops are small, frequent and everywhere. The picture is therefore not a quantitative claim about the process.
* **Invisible.** Anything shorter than one sample, anything longer than the window, the 47 smaller Fourier terms, and every process below the variance of the seventh planet (they are stars; a quiet process's brightness is only its memory).
* **Stars are not positioned by data.** Angle and radius come from the identity hash; only brightness (memory) and the flare (birth) mean something.

## Try it

```sh
./target/release/isotop --demo --view ptolemy --time 300 --width 1600 --output /tmp/ptolemy.png --renderer cpu
./target/release/isotop --demo --view ptolemy --time 12  --width 1600 --output /tmp/forming.png   # the heavens still forming
./target/release/isotop --view ptolemy            # live; v opens the picker, Tab cycles, click a planet for its sphere and epicycle count
```

Same-seed renders are repeatable: the demo is deterministic in `--time`, and the trail is a function of the clock. Compare `--time 300` and `--time 301`: the red stretches shift while the chains' shapes barely change, as the sliding window predicts.

## Where in the code

* `src/ptolemy.rs`: `Ptolemy` (the `ViewModule`), `observe`, `measure`, `signal`, `epicycles`, `scaled`, `Deferent`, `Planet`, `retrograde`, `star_seat`, `draw_planet_paths`, `draw_armillary`, `stroke`; constants `SKY`, `RADII`, `SPHERES`, `KEEP_RANK`, `FORMING`, `TERMS`, `GAIN`, `REACH`, `OUTER_PERIOD`, `REPLAY`, `RESORT`, `EASE`, `TRAIL_*`, `STAR_RING`, `STAR_TURN`, `FLASH_WINDOW`.
* `src/simulation.rs`: `Complex`, `dft`, `smoothstep`; `src/history.rs`: `History::recent`, `History::changes`, `SERIES`; `src/stellar.rs`: `magnitude`, `spread`, `flash`, `Frame::star`.
* `src/views.rs`: the `ptolemy` registry row (`Backdrop::Space`, `haze: false`, `flash: false`, demo options `churn` and `observer`, harness `time: 300`).
* Tests: `dft_reconstruction_equals_the_samples_at_integer_times`, `a_planet_whose_epicycle_outruns_its_deferent_is_in_retrograde_and_a_slow_one_is_not`, `wanderers_keep_their_spheres_while_ranks_shuffle_inside_the_top_ten`, `a_wanderer_leaves_its_sphere_when_it_drops_out_of_the_top_ten_or_exits`.
* Design source: `docs/briefs/views-2026-10.md` (section 1, `ptolemy`) and `docs/mockups/ptolemy.jpg`. There is no plugin system: this is a compile-time module and one `REGISTRY` row.

## Variations and open questions

1. **Real periods.** Replace the Kepler-law deferent with one whose period is the process's own dominant period (the largest term of the spectrum): the planet would complete a deferent revolution once per cycle of its main behaviour.
2. **Equant and eccentric.** Add Ptolemy's equant to the deferent and compare retrograde statistics. Does the bisected eccentricity make the loops more regular?
3. **Hann-windowed minute.** Remove the wrap discontinuity (see the Gibbs figure on the Math page) and see whether planets' tips then pass through the newest sample.
4. **Colour by handedness.** Draw clockwise and counter-clockwise epicycles differently.
5. **A planet's second chain.** A second DFT of the *memory* alone, drawn as a small moon on the CPU epicycle: separates the two physical signals that the single complex chain entangles.
6. **Open:** is the variance score the right "most varied" measure? It adds a variance in cores to a variance in doublings, two incommensurable units; a normalised version would rank differently.

## References

* G. J. Toomer (transl.), *Ptolemy's Almagest* (London: Duckworth, 1984).
* O. Neugebauer, *A History of Ancient Mathematical Astronomy* (Springer, 1975).
* N. R. Hanson, "The mathematical power of epicyclical astronomy", *Isis* 51 (1960), 150-158.
* J. Ferguson, *Astronomy Explained upon Sir Isaac Newton's Principles* (London, 1756), the plates of apparent planetary paths that the mockup follows (named in `docs/briefs/views-2026-10.md`).
* [Math Fourier Epicycles](Math-Fourier-Epicycles), [Math Stellar Photometry](Math-Stellar-Photometry), [Theory History](Theory-History), [Theory Determinism](Theory-Determinism).
