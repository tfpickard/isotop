"""Shared maths for the Fourier-epicycle figures: the demo workload's CPU and memory of one process
(model.rs, demo_with), the direct DFT of src/simulation.rs, and the epicycle selection of
src/ptolemy.rs (the K largest terms without the mean, frequencies folded to -N/2 < k <= N/2 - 1).
Standard library only.
"""

import math

TAU = 2 * math.pi
SERIES = 60


def demo_cpu(i, t):
    """CPU percent of demo process i at time t (model.rs, demo_with)."""
    phase = t * 0.55 + i * 1.71
    cpu = (math.sin(phase) * 0.5 + 0.5) ** 4 * (20.0 + (i % 9) * 24.0)
    return cpu + (110.0 if i % 47 == 0 else 0.0)


def demo_memory(i, t):
    """Resident bytes of demo process i at time t."""
    phase = t * 0.55 + i * 1.71
    return (12.0 + (i * 71 % 900) * (0.8 + 0.2 * math.sin(phase * 0.2))) * 1048576.0


def doublings(memory):
    """Memory in doublings: log2 of MiB, floored at 1 MiB (ptolemy.rs, doublings)."""
    return math.log2(max(memory / 1048576.0, 1.0))


def window(i, start, count=SERIES):
    """The process's last `count` one-second samples as cores + i * doublings, mean removed."""
    z = [complex(demo_cpu(i, start + n) / 100.0, doublings(demo_memory(i, start + n)))
         for n in range(count)]
    mean = sum(z) / len(z)
    return [v - mean for v in z], mean


def dft(z):
    n = len(z)
    return [sum(z[m] * complex(math.cos(TAU * k * m / n), -math.sin(TAU * k * m / n))
                for m in range(n)) for k in range(n)]


def epicycles(z, keep):
    """[(frequency, coefficient)] of the `keep` largest terms without the mean, as ptolemy.rs
    epicycles() selects them (ties by index), with the window starting at sample 0."""
    n = len(z)
    spectrum = dft(z)
    order = sorted(range(1, n), key=lambda k: (-abs(spectrum[k]), k))[:keep]
    return [((k if k < n // 2 else k - n), spectrum[k] / n) for k in order]


def evaluate(terms, t, n=SERIES):
    """The chain's tip at sample time t (fractional allowed): sum of c e^{2 pi i f t / N}."""
    return sum(c * complex(math.cos(TAU * f * t / n), math.sin(TAU * f * t / n))
               for f, c in terms)


def rms_error(z, terms):
    n = len(z)
    return math.sqrt(sum(abs(z[m] - evaluate(terms, m, n)) ** 2 for m in range(n)) / n)
