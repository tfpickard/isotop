#!/usr/bin/env python3
"""Aliasing of a periodic load seen through one counter delta per second.

Run from the repository root: python3 docs/figures/model-sampling-alias.py
Writes docs/media/figures/model-sampling-alias.svg.

A load x(t) = 50 + 40 sin(2 pi f t) percent of a core is read as the mean over each dt = 1 s
interval (counter delta / dt), then filtered with tau = 1.5 s, as `model::measure` does.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sampling_plot import BLUE, GREEN, INK, MUTED, ORANGE, PURPLE, Figure  # noqa: E402

TAU = 1.5
DT = 1.0
F = 1.1
MEAN, AMPLITUDE = 50.0, 40.0


def sinc(x):
    return 1.0 if x == 0 else math.sin(math.pi * x) / (math.pi * x)


def load(t):
    return MEAN + AMPLITUDE * math.sin(2 * math.pi * F * t)


def interval_mean(start, end):
    """Exact mean of the load over an interval."""
    omega = 2 * math.pi * F
    return MEAN + AMPLITUDE * (math.cos(omega * start) - math.cos(omega * end)) / (omega * (end - start))


def folded(f):
    """The frequency a load at f appears at after sampling every DT seconds."""
    rate = 1 / DT
    return abs(f - rate * round(f / rate))


def main():
    figure = Figure(
        900,
        600,
        "Aliasing of a periodic load",
        "A 1.1 hertz load seen once per second appears as a 0.1 hertz wobble of a few percent.",
    )
    figure.text(60, 28, "A 1.1 Hz load, read once a second", size=15, weight="600")
    top = figure.axes(
        72, 62, 770, 190, (0, 30), (0, 100), xlabel="time (s)", ylabel="CPU (% of one core)",
        yticks=[0, 25, 50, 75, 100], title="true load, interval mean (raw reading) and displayed CPU",
    )
    ts = [i * 0.01 for i in range(0, 3001)]
    top.path(ts, [load(t) for t in ts], MUTED, 0.7, None, 0.7)
    starts = [float(k) for k in range(0, 30)]
    raws = [interval_mean(k, k + DT) for k in range(0, 30)]
    top.steps(starts, raws, ORANGE, 1.4)
    value = MEAN
    a = math.exp(-DT / TAU)
    outs = []
    for raw in raws:
        value += (1 - a) * (raw - value)
        outs.append(value)
    top.points([k + DT for k in range(30)], outs, BLUE, 3.0)
    top.path([k + DT for k in range(30)], outs, BLUE, 1.5)
    top.legend([(MUTED, "true load, 50 +/- 40 at 1.1 Hz", None), (ORANGE, "raw: mean over each second", None), (BLUE, "displayed CPU", None)], x=top.left + 560, y=top.top + 18)

    gain = figure.axes(
        72, 312, 360, 170, (0, 3), (0, 100), xlabel="load frequency (Hz)", ylabel="% of the oscillation passed",
        title="interval mean: |sinc(f dt)|", yticks=[0, 25, 50, 75, 100], xticks=[0, 0.5, 1, 1.5, 2, 2.5, 3],
        xformat=lambda v: f"{v:g}",
    )
    fr = [i * 3.0 / 900 for i in range(901)]
    gain.path(fr, [100 * abs(sinc(f * DT)) for f in fr], ORANGE, 1.8)
    gain.vline(0.5, MUTED, "4 4")
    gain.points([F], [100 * abs(sinc(F * DT))], PURPLE, 3.6)
    gain.note(F, 100 * abs(sinc(F * DT)), f"{100 * abs(sinc(F * DT)):.1f}% at 1.1 Hz", color=PURPLE, dx=8, dy=-12)

    fold = figure.axes(
        500, 312, 340, 170, (0, 3), (0, 0.55), xlabel="true frequency (Hz)", ylabel="apparent frequency (Hz)",
        title="where it lands after sampling at 1 Hz", yticks=[0, 0.1, 0.25, 0.5], xticks=[0, 0.5, 1, 1.5, 2, 2.5, 3],
        xformat=lambda v: f"{v:g}", yformat=lambda v: f"{v:g}",
    )
    xs = [i * 3.0 / 1500 for i in range(1501)]
    segments, current = [], []
    previous = None
    for f in xs:
        y = folded(f)
        if previous is not None and abs(y - previous) > 0.3:
            segments.append(current)
            current = []
        current.append((f, y))
        previous = y
    segments.append(current)
    for segment in segments:
        fold.path([p[0] for p in segment], [p[1] for p in segment], GREEN, 1.8)
    fold.points([F], [folded(F)], PURPLE, 3.6)
    fold.note(F, folded(F), "1.1 Hz reads as 0.1 Hz", color=PURPLE, dx=8, dy=-8)

    measured = max(outs[10:]) - min(outs[10:])
    figure.text(
        72, 556, f"Displayed wobble after settling: {measured:.1f} points peak to peak with a period of {1 / folded(F):.0f} s, from a load that swings 80 points peak to peak.",
        size=11.5, color=MUTED,
    )
    figure.text(72, 574, "Nothing in the number says it is not a slow, real fluctuation: a counter delta cannot tell a 0.1 Hz load from a 1.1 Hz one.", size=11.5, color=MUTED)
    out = os.path.join("docs", "media", "figures", "model-sampling-alias.svg")
    figure.save(out)
    print("wrote", out, f"(wobble {measured:.2f} points peak to peak)")


if __name__ == "__main__":
    main()
