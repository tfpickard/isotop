#!/usr/bin/env python3
"""Rate estimation from a quantised cumulative counter, and what the smoothing does to the noise.

Run from the repository root: python3 docs/figures/model-quantised-counters.py
Writes docs/media/figures/model-quantised-counters.svg.

/proc/<pid>/stat reports utime + stime in USER_HZ ticks (q = 10 ms at USER_HZ = 100). A rate is the
difference of two readings, so each reading is off by up to one tick and the difference by up to
one tick either way. For a load whose busy fraction per interval is uniform in [0, 1] the error of
the difference has variance q^2 / 6 (the sum of two independent uniform residues), so
    sigma_raw = 100 * q / (dt * sqrt(6))     percent of a core.
The exponential filter turns the error sequence e[k] = f[k-1] - f[k] (residues f) into
    sigma_ema = sigma_raw * (1 - a) / sqrt(1 + a),   a = exp(-dt / tau),
which for dt << tau tends to 100 * q / (tau * sqrt(12)), independent of dt.
"""

import math
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sampling_plot import BLUE, GREEN, INK, MUTED, ORANGE, PURPLE, Figure  # noqa: E402

TAU = 1.5
Q = 0.01  # one USER_HZ tick, seconds


def series(load, dt, seconds, phase=0.123):
    """Raw and smoothed readings of a process that really uses `load` of one core, after the
    filter has settled."""
    a = math.exp(-dt / TAU)
    steps = int(round(seconds / dt))
    previous = math.floor(phase)
    times, raws, outs = [], [], []
    value = load * 100
    for k in range(1, steps + 1):
        t = k * dt
        counter = math.floor((load * t + phase * Q) / Q)  # the kernel truncates to whole ticks
        raw = (counter - previous) * Q / dt * 100
        previous = counter
        value += (1 - a) * (raw - value)
        times.append(t)
        raws.append(raw)
        outs.append(value)
    return times, raws, outs


def monte_carlo(dt, loads=160, steps=500, seed=7):
    """rms error of the raw readings and of the smoothed ones over random constant loads."""
    rng = random.Random(seed)
    a = math.exp(-dt / TAU)
    raw_square = ema_square = 0.0
    count = 0
    for _ in range(loads):
        load = rng.uniform(0.05, 0.95)
        phase = rng.random()
        previous = 0
        value = load * 100
        for k in range(1, steps + 1):
            counter = math.floor((load * k * dt + phase * Q) / Q)
            if k == 1:
                previous = counter
                continue
            raw = (counter - previous) * Q / dt * 100
            previous = counter
            value += (1 - a) * (raw - value)
            if k > 60:
                raw_square += (raw - load * 100) ** 2
                ema_square += (value - load * 100) ** 2
                count += 1
    return math.sqrt(raw_square / count), math.sqrt(ema_square / count)


def main():
    figure = Figure(
        900,
        480,
        "Rates from a quantised counter",
        "Raw and smoothed CPU readings of a steady load at 0.1 s sampling, and the error of both against the sampling interval.",
    )
    figure.text(60, 28, "A rate is a difference of whole ticks (USER_HZ = 100, tau = 1.5 s)", size=15, weight="600")
    left = figure.axes(
        72, 62, 380, 320, (0, 12), (30, 44), xlabel="time (s)", ylabel="CPU reading (% of one core)",
        title="a steady 37.3% load, dt = 0.1 s, filter settled", yticks=[30, 33, 36, 37.3, 40, 43],
        yformat=lambda v: f"{v:g}",
    )
    times, raws, outs = series(0.373, 0.1, 12.0)
    left.hline(37.3, MUTED, "3 5")
    left.steps([0.0] + times[:-1], raws, ORANGE, 1.1)
    left.path(times, outs, BLUE, 2.2)
    left.legend([(ORANGE, "raw: 3 or 4 ticks per 0.1 s", None), (BLUE, "displayed (filtered)", None)], x=left.left + 12, y=left.top + 18)

    right = figure.axes(
        540, 62, 320, 320, (0.1, 10), (0.03, 10), xlog=True, ylog=True,
        xlabel="sampling interval dt (s)", ylabel="rms error (% of one core)",
        title="error against the sampling interval", xticks=[0.1, 0.2, 0.5, 1, 2, 5, 10],
        yticks=[0.05, 0.1, 0.2, 0.5, 1, 2, 5], xformat=lambda v: f"{v:g}", yformat=lambda v: f"{v:g}",
    )
    dts = [10 ** (-1 + i * 2 / 200) for i in range(201)]
    raw_theory = [100 * Q / (dt * math.sqrt(6)) for dt in dts]
    ema_theory = [r * (1 - math.exp(-dt / TAU)) / math.sqrt(1 + math.exp(-dt / TAU)) for r, dt in zip(raw_theory, dts)]
    right.path(dts, raw_theory, ORANGE, 1.8)
    right.path(dts, ema_theory, BLUE, 1.8)
    plateau = 100 * Q / (TAU * math.sqrt(12))
    right.hline(plateau, MUTED, "3 5")
    right.note(0.1, plateau, f"dt << tau: {plateau:.2f}", color=MUTED, dx=6, dy=-6)
    for dt in (0.1, 0.25, 0.5, 1.0, 2.0, 5.0):
        raw_rms, ema_rms = monte_carlo(dt)
        right.points([dt], [raw_rms], ORANGE, 3.2)
        right.points([dt], [ema_rms], BLUE, 3.2)
    right.legend([(ORANGE, "raw reading", None), (BLUE, "displayed (filtered)", None)], x=right.left + 12, y=right.top + 18)
    right.note(1.0, 100 * Q / math.sqrt(6), "0.41%", color=ORANGE, dx=8, dy=-8)
    figure.text(
        72, 430, "Lines: theory for a busy fraction per interval uniform in [0, 1]. Dots: simulation of random steady loads (seed 7).",
        size=11.5, color=MUTED,
    )
    figure.text(
        72, 447, "The raw error grows as 1 / dt; the displayed error stays near 0.19% for every dt below tau.",
        size=11.5, color=MUTED,
    )
    figure.text(
        72, 464, "The counter is cumulative, so a tick is never lost, only moved to the next interval.",
        size=11.5, color=MUTED,
    )
    out = os.path.join("docs", "media", "figures", "model-quantised-counters.svg")
    figure.save(out)
    print("wrote", out)


if __name__ == "__main__":
    main()
