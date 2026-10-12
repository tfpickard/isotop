#!/usr/bin/env python3
"""Step response of the CPU smoothing in `model::measure` at the real time constant.

Run from the repository root: python3 docs/figures/model-smoothing-step.py
Writes docs/media/figures/model-smoothing-step.svg.

The filter is `cpu = previous + smoothing(dt, 1.5) * (raw - previous)` with
`smoothing(dt, tau) = 1 - exp(-dt / tau)`, where `raw` is the average load over the last
interval (a counter delta divided by dt).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sampling_plot import BLUE, GREEN, INK, MUTED, ORANGE, PURPLE, Figure  # noqa: E402

TAU = 1.5
STEP_AT = 0.37  # the load jumps from 0 to 100% here, between two samples


def response(dt, horizon):
    """(sample times, raw box averages, smoothed values) for a step at STEP_AT."""
    alpha = 1 - math.exp(-dt / TAU)
    times, raws, outs = [], [], []
    value = 0.0
    k = 1
    while k * dt <= horizon + 1e-9:
        start, end = (k - 1) * dt, k * dt
        busy = max(0.0, end - max(start, STEP_AT)) / dt
        raw = 100 * busy
        value += alpha * (raw - value)
        times.append(end)
        raws.append(raw)
        outs.append(value)
        k += 1
    return times, raws, outs


def main():
    figure = Figure(
        880,
        470,
        "Step response of the CPU smoothing",
        "A load step seen through the interval average and the exponential filter with a 1.5 second time constant.",
    )
    figure.text(60, 28, "A load step through isotop's CPU filter (tau = 1.5 s)", size=15, weight="600")
    axes = figure.axes(
        72, 62, 770, 330, (0, 10), (0, 105), xlabel="time since the start of the run (s)",
        ylabel="displayed CPU (% of one core)", yticks=[0, 20, 40, 60, 80, 100],
    )
    # The load itself and the ideal continuous filter.
    axes.path([0, STEP_AT, STEP_AT, 10], [0, 0, 100, 100], MUTED, 1.4, "2 4")
    ts = [STEP_AT + i * 0.02 for i in range(0, 483)]
    axes.path(ts, [100 * (1 - math.exp(-(t - STEP_AT) / TAU)) for t in ts], INK, 1.3, "6 4", 0.8)
    # dt = 1 s: the raw interval averages and the filter output.
    t1, raw1, out1 = response(1.0, 10)
    axes.steps([0.0] + t1[:-1], [0.0] + raw1[:-1], ORANGE, 1.6)
    axes.points(t1, out1, BLUE, 4.2)
    axes.path([0.0] + t1, [0.0] + out1, BLUE, 1.3, None, 0.6)
    # dt = 0.25 s.
    t2, raw2, out2 = response(0.25, 10)
    axes.points(t2, out2, GREEN, 1.9, 0.95)
    # Landmarks: one and three time constants after the step.
    for k, label in ((1, "63.2%"), (3, "95.0%")):
        t = STEP_AT + k * TAU
        axes.vline(t, PURPLE, "2 5", 1.1)
        level = 100 * (1 - math.exp(-k))
        axes.points([t], [level], PURPLE, 3.4)
        axes.note(t, level, f"{label} at {k} tau", color=PURPLE, dx=8, dy=18)
    axes.note(STEP_AT, 100, "load steps 0 to 100% at 0.37 s", dx=8, dy=-7, color=MUTED)
    axes.legend(
        [
            (MUTED, "true load", "2 4"),
            (INK, "ideal continuous filter, 1 - exp(-t / tau)", "6 4"),
            (ORANGE, "raw reading: counter delta / dt, dt = 1 s", None),
            (BLUE, "displayed CPU, dt = 1 s (dots)", None),
            (GREEN, "displayed CPU, dt = 0.25 s (dots)", None),
        ],
        x=axes.left + 470,
        y=axes.top + 224,
    )
    figure.text(
        72, 442, "At every sample instant the filter equals the continuous curve exactly, whatever dt is; dt only sets how often it is read.",
        size=11.5, color=MUTED,
    )
    figure.text(72, 459, "The first raw reading after the step is partial (here 63% for the 0.37 s offset): the interval average also delays the edge by dt / 2 on average.", size=11.5, color=MUTED)
    out = os.path.join("docs", "media", "figures", "model-smoothing-step.svg")
    figure.save(out)
    print("wrote", out)


if __name__ == "__main__":
    main()
