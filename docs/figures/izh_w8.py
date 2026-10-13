"""Izhikevich (2003) neurons in pure Python, for the Math-Spiking-Neurons and Planned-Cortex figures.

    v' = 0.04 v^2 + 5 v + 140 - u + I,   u' = a (b v - u),   if v >= 30: v <- c, u <- u + d
integrated with Euler steps of 0.5 ms (time in ms, as in the paper)."""

import math

TYPES = {  # name: (a, b, c, d), Izhikevich 2003, Fig. 2 and Table
    "RS": (0.02, 0.2, -65.0, 8.0),
    "IB": (0.02, 0.2, -55.0, 4.0),
    "CH": (0.02, 0.2, -50.0, 2.0),
    "FS": (0.1, 0.2, -65.0, 2.0),
    "LTS": (0.02, 0.25, -65.0, 2.0),
    "TC": (0.02, 0.25, -65.0, 0.05),
    "RZ": (0.1, 0.26, -65.0, 2.0),
}
DT = 0.5
MASK = (1 << 64) - 1


class Normal:
    """splitmix64 plus Box-Muller, deterministic."""

    def __init__(self, seed):
        self.state = seed & MASK

    def unit(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        z ^= z >> 31
        return (z >> 40) / 16777216.0

    def normal(self):
        return math.sqrt(-2.0 * math.log(1.0 - self.unit())) * math.cos(2.0 * math.pi * self.unit())


def simulate(kind, current, duration_ms, noise=0.0, seed=1, record=False, current_of_time=None):
    """Returns (spike times in ms, membrane trace list of (t, v) when `record`)."""
    a, b, c, d = TYPES[kind]
    v, u = -65.0, b * -65.0
    rng = Normal(seed) if noise > 0 else None
    spikes, trace = [], []
    steps = int(round(duration_ms / DT))
    for s in range(steps):
        t = s * DT
        if v >= 30.0:
            spikes.append(t)
            if record:
                trace.append((t, 30.0))
            v, u = c, u + d
        i = current_of_time(t) if current_of_time else current
        if rng is not None and i > 0:
            i += noise * rng.normal()
        v += DT * (0.04 * v * v + 5.0 * v + 140.0 - u + i)
        u += DT * a * (b * v - u)
        if v > 30.0:
            v = 30.0 + 1e-3
        if record:
            trace.append((t, min(v, 30.0)))
    return spikes, trace


def rate_hz(kind, current, noise=0.0, seed=1, settle_ms=1000.0, window_ms=2000.0):
    spikes, _ = simulate(kind, current, settle_ms + window_ms, noise=noise, seed=seed)
    return sum(1 for t in spikes if t >= settle_ms) / (window_ms / 1000.0)


def calibrate(kind, step=0.05, top=40.0):
    """Noise-free rheobase (smallest I with >= 1 Hz) and the I that gives 40 Hz, as in issue #52."""
    rheobase = i40 = None
    previous = (0.0, 0.0)
    i = 0.0
    while i <= top:
        rate = rate_hz(kind, i)
        if rheobase is None and rate >= 1.0:
            rheobase = i
        if i40 is None and rate >= 40.0:
            lo_i, lo_r = previous
            i40 = lo_i + (40.0 - lo_r) / (rate - lo_r) * (i - lo_i) if rate > lo_r else i
        previous = (i, rate)
        if rheobase is not None and i40 is not None:
            break
        i += step
    return rheobase, i40
