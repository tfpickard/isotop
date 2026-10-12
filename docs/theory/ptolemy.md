# Ptolemy: epicycles are a Fourier series

**View:** `ptolemy` · **Code:** `src/ptolemy.rs`, `simulation::dft` · **Prototype:** `docs/mockups/code/ptolemy.py`

## What you are looking at

| On screen | Measured quantity |
|---|---|
| Earth at the centre | isotop itself |
| Seven planets | the seven processes whose CPU and memory *varied most* over the last minute |
| A planet's deferent (big circle) | its sphere, assigned by rank. Radius and period are decoration; the busiest wanderer takes the innermost sphere |
| A planet's chain of epicycles | the 12 largest Fourier terms of its last 60 samples of CPU and memory |
| Where the chain's tip goes | the last minute of that process, replayed once per minute |
| Red stretches of a trail | the planet is in *retrograde* as seen from Earth |
| Fixed stars on a ring that turns once a minute | every other process; brightness is memory |
| A nova or a fade | a birth or an exit |

The epicycle chain is the data. The deferent is a vehicle for the picture.

## Background theory

### The historical model

Greek astronomy had to account for planets that usually move eastward against the stars but periodically stop, move west for weeks (*retrograde motion*) and resume. In Ptolemy's *Almagest* (2nd century CE) each planet rides a small circle, the **epicycle**, whose centre rides a large circle about the Earth, the **deferent**. With the right radii and speeds the planet's apparent path loops back on itself at the right moments. The model stayed usefully accurate for over a thousand years, which is an excellent record for a wrong cosmology.

In the heliocentric picture retrograde is a perspective effect: the Earth, on a smaller and faster orbit, overtakes Mars or Jupiter and the outer planet seems briefly to slide backward against the stars, like a slower car seen from a faster one on a bend. The epicycle on a Ptolemaic outer planet is the Earth's own orbit in disguise, which is why its period is always one year.

### Epicycles are a Fourier series

A path in the plane is a complex-valued function $z(t) = x(t) + i\,y(t)$. For a periodic path with period $T$,

$$z(t) = \sum_{k=-\infty}^{\infty} c_k\, e^{2\pi i k t / T}$$

and every term $c_k e^{2\pi i k t/T}$ is a point travelling uniformly on a circle of radius $|c_k|$ at $k$ turns per period. Chaining the terms, each circle centred on the tip of the one before, gives a pendulum of rotating arms whose end draws $z(t)$. That is an epicycle chain. Negative $k$ run the other way, so a path needs both signs and the arms do not all turn in the same direction.

So epicycles are not wrong as a *description*: any smooth periodic path can be approximated to any accuracy by enough of them. The model is predictive without being explanatory. It fits whatever you feed it, which is why a good fit to data is not evidence for the mechanism behind it.

### The discrete Fourier transform

Given $N$ samples $z[n]$, the DFT is

$$X[k] = \sum_{n=0}^{N-1} z[n]\, e^{-2\pi i k n / N}$$

and the samples are recovered exactly by $z[n] = \frac{1}{N}\sum_k X[k]\, e^{+2\pi i k n/N}$. The coefficient of the $k$-th arm is $X[k]/N$. Frequencies above $N/2$ are the same as negative frequencies, so the code relabels them $k - N$.

Keeping only the largest few terms is lossy compression. It is the same idea as JPEG or MP3 in a different domain. The terms with the greatest magnitude carry the most of the signal's energy (Parseval: $\sum |z|^2 = \tfrac{1}{N}\sum |X|^2$).

### Retrograde, computed

Seen from Earth at the origin, a body at position $p$ has apparent longitude $\lambda = \arg p$. Writing $p$ as a complex number,

$$\frac{d\lambda}{dt} = \frac{\operatorname{Im}\left(\bar p\, p'\right)}{|p|^2}$$

so a body is in retrograde exactly when $\operatorname{Im}(\bar p\,p') < 0$, that is when the cross product of position and velocity points clockwise. `retrograde` is that one line.

A planet moves backward when the epicycle's own velocity, directed against the deferent's, outweighs the deferent's. Fast, large epicycles on a slow deferent produce loops. A smooth, slow process produces none.

### Kepler's third law

Inner planets really do go faster. The deferent period follows $T \propto r^{3/2}$ from the outermost sphere (1,200 s at the outer sphere), so the sphere ordering has the visual rhythm of a solar system.

## How isotop implements it

1. **The signal.** For each process, a complex series $z[n] = \text{CPU in cores} + i\log_2(\text{memory in MiB})$ over the last 60 samples (`history::SERIES`), minus its mean. The mean is the DC term and is left out; it would only offset the whole chain. Samples missing because the process was younger than the window are padded by holding its first reading; a gap holds its last.
2. **Who is a planet.** The variance of cores plus the variance of memory in doublings, both over the window. A process needs 30 samples (`FORMING`) before it can rank. The top seven by variance get the seven spheres, named Moon, Mercury, Venus, Sun, Mars, Jupiter, Saturn from the inside out, busiest innermost. An incumbent keeps its sphere while it stays in the top 10, so ranks can shuffle without the planets jumping.
3. **The chain.** `epicycles` runs a direct $O(N^2)$ DFT (60 samples need no padding or radix games), sorts by magnitude, keeps 12, and rotates each coefficient back by its phase at the window's first sample. The back-rotation is the trick that makes the coefficients nearly constant when the window slides by one sample, so the epicycles drift smoothly instead of the whole chain reshuffling every second. Coefficients ease to each new sample over the measured sample interval.
4. **Replay.** The chain plays the window once every 60 s (`REPLAY`), regardless of the sample rate.
5. **Scale.** One global gain turns cores and doublings into world units, soft-clamped so the chain's reach never exceeds $L$: with $T = K\sum|c_k|$, the gain is $K L \tanh(T/L)/T$, which approaches $K$ for small chains and caps long ones at length $L$. A wild process cannot swallow the sky.
6. **Stars.** Every non-planet is a fixed star at a hashed angle and radius on a ring that turns once per 60 s. Magnitude comes from memory, $m = -2.5\log_{10}(\text{mem}/1\,\text{GiB})$ (the same definition as the HR diagram view; `stellar.rs`). A new process is a nova; an exiting one fades.

## Faithful, stylized, invented

- **Faithful:** the DFT; the reconstruction identity, which a test checks (`dft_reconstruction_equals_the_samples_at_integer_times`); the retrograde condition; Kepler's scaling for the deferent period.
- **Stylized:** the real part of $z$ is CPU and the imaginary part is memory, which treats two unrelated quantities as the two axes of a plane. Their units are made commensurable only by the choice of gain.
- **Invented:** the deferent. The data lives entirely in the chain. The deferent is a slow circle on which to hang it, and the apparent path of a planet (deferent plus chain) is a combination with no direct meaning. Whether a planet "loops" depends on the deferent's period, so retrograde stretches are a feature of the picture. They are real in the sense that they are computed from what you see, and not a property of the process.
- **Not modelled:** the equant, Ptolemy's off-centre point for uniform motion, which fixed the speed anomalies of real planets.

## Reading the picture

- A planet whose chain is a big slow loop is a process with a large-amplitude slow swing: a build that ramps and ends, a GC sawtooth.
- Many small fast arms is noise or a tight scheduling loop.
- A near-motionless chain is flat CPU and memory, which is a bad candidate for a planet; it only ranks because the system is quiet.
- The tip returning to its start after exactly one minute is the periodic replay, not the process repeating.

## Further reading

- Ptolemy, *Almagest* (trans. Toomer, 1984). Book IX covers the planetary models, including the equant.
- Hanson, N. R. (1960), "The Mathematical Power of Epicyclical Astronomy", *Isis* 51, 150: the Fourier argument, made early.
- Oppenheim & Schafer, *Discrete-Time Signal Processing*, ch. 8 (the DFT) for conventions and Parseval.
- Brigham, *The Fast Fourier Transform*, for the $O(N \log N)$ alternative isotop does not need at $N = 60$.
