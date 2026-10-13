"""Hodgkin-Huxley (1952) in the modern convention (V in mV, rest near -65 mV), RK4, standard library."""

import math

C, GNA, GK, GL = 1.0, 120.0, 36.0, 0.3
ENA, EK, EL = 50.0, -77.0, -54.387


def rates(v):
    def safe(x, scale):  # x / (1 - exp(-x / scale)) without the removable singularity at x = 0
        return scale if abs(x) < 1e-7 else x / (1.0 - math.exp(-x / scale))
    an = 0.01 * safe(v + 55.0, 10.0)
    bn = 0.125 * math.exp(-(v + 65.0) / 80.0)
    am = 0.1 * safe(v + 40.0, 10.0)
    bm = 4.0 * math.exp(-(v + 65.0) / 18.0)
    ah = 0.07 * math.exp(-(v + 65.0) / 20.0)
    bh = 1.0 / (1.0 + math.exp(-(v + 35.0) / 10.0))
    return an, bn, am, bm, ah, bh


def derivative(state, current):
    v, n, m, h = state
    an, bn, am, bm, ah, bh = rates(v)
    dv = (current - GNA * m ** 3 * h * (v - ENA) - GK * n ** 4 * (v - EK) - GL * (v - EL)) / C
    return (dv, an * (1 - n) - bn * n, am * (1 - m) - bm * m, ah * (1 - h) - bh * h)


def rest_state(v=-65.0):
    an, bn, am, bm, ah, bh = rates(v)
    return [v, an / (an + bn), am / (am + bm), ah / (ah + bh)]


def run(current_of_time, duration, dt=0.01, state=None):
    """Returns (times, V, n, m, h) sampled every step."""
    state = list(state or rest_state())
    for _ in range(int(20.0 / dt)):  # settle to the true rest point first
        state = rk4(state, 0.0, dt)
    out = ([], [], [], [], [])
    steps = int(round(duration / dt))
    for s in range(steps + 1):
        t = s * dt
        out[0].append(t)
        out[1].append(state[0])
        out[2].append(state[1])
        out[3].append(state[2])
        out[4].append(state[3])
        state = rk4(state, current_of_time(t), dt)
    return out


def rk4(state, current, dt):
    k1 = derivative(state, current)
    k2 = derivative([s + 0.5 * dt * k for s, k in zip(state, k1)], current)
    k3 = derivative([s + 0.5 * dt * k for s, k in zip(state, k2)], current)
    k4 = derivative([s + dt * k for s, k in zip(state, k3)], current)
    return [s + dt / 6.0 * (a + 2 * b + 2 * c + d) for s, a, b, c, d in zip(state, k1, k2, k3, k4)]


def firing_rate(current, duration=1000.0, dt=0.02):
    """Spikes per second counted as upward crossings of 0 mV in the second half of the run."""
    t, v, *_ = run(lambda _: current, duration, dt)
    half = len(t) // 2
    spikes = sum(1 for i in range(half, len(v)) if v[i - 1] < 0.0 <= v[i])
    return spikes / ((duration / 2) / 1000.0)
