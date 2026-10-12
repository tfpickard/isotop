#!/usr/bin/env python3
"""How many processes a sampler ever sees, against how long they live.

Run from the repository root: python3 docs/figures/life-survival-sampling.py
Writes docs/media/figures/life-survival-sampling.svg.

Samples are taken at instants dt apart. A process that lives L seconds, born at a uniformly
random moment, straddles at least one instant with probability min(L / dt, 1). For lifetimes
drawn from an exponential distribution with mean m the expected fraction seen is
    E[min(L / dt, 1)] = (m / dt) * (1 - exp(-dt / m)).
"""

import math
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sampling_plot import BLUE, GREEN, INK, MUTED, ORANGE, PURPLE, Figure  # noqa: E402


def seen_fixed(ratio):
    """ratio = L / dt."""
    return min(ratio, 1.0)


def seen_exponential(ratio):
    """ratio = mean lifetime / dt."""
    return ratio * (1 - math.exp(-1 / ratio))


def monte_carlo(ratio, trials=40000, seed=11):
    """Fraction of exponential lifetimes with mean `ratio` (dt = 1) that straddle an instant."""
    rng = random.Random(seed)
    seen = 0
    for _ in range(trials):
        birth = rng.random()
        lifetime = rng.expovariate(1 / ratio)
        # The next instant at or after the birth is at 1.0 (instants at integers, birth in [0, 1)).
        if birth + lifetime >= 1.0:
            seen += 1
    return seen / trials


def main():
    figure = Figure(
        760,
        470,
        "Fraction of processes a sampler sees",
        "Fraction of processes observed by interval sampling against the ratio of their mean lifetime to the sampling interval.",
    )
    figure.text(60, 28, "Who is alive at a sampling instant", size=15, weight="600")
    axes = figure.axes(
        72, 62, 640, 330, (0.01, 100), (0, 1.02), xlog=True,
        xlabel="lifetime / sampling interval (log scale)", ylabel="fraction of processes ever sampled",
        xticks=[0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 20, 50, 100], xformat=lambda v: f"{v:g}",
        yticks=[0, 0.25, 0.5, 0.75, 1], yformat=lambda v: f"{v:g}",
    )
    ratios = [10 ** (-2 + i * 4 / 400) for i in range(401)]
    axes.path(ratios, [seen_fixed(r) for r in ratios], ORANGE, 1.8, "6 4")
    axes.path(ratios, [seen_exponential(r) for r in ratios], BLUE, 2.2)
    sample_ratios = [0.02, 0.05, 0.1, 0.3, 1, 3, 10, 30]
    axes.points(sample_ratios, [monte_carlo(r) for r in sample_ratios], GREEN, 3.4)
    axes.points([1.0], [seen_exponential(1.0)], PURPLE, 4.0)
    axes.note(1.0, seen_exponential(1.0), f"mean lifetime = dt: {100 * seen_exponential(1.0):.0f}% seen", color=PURPLE, dx=10, dy=18)
    axes.points([0.1], [seen_exponential(0.1)], PURPLE, 4.0)
    axes.note(0.1, seen_exponential(0.1), "a 0.1 s average life at dt = 1 s: 10% seen", color=PURPLE, dx=10, dy=-10)
    axes.legend(
        [(ORANGE, "every process lives exactly L", "6 4"), (BLUE, "exponential lifetimes, mean L", None), (GREEN, "simulation, 40 000 processes per dot", None)],
        x=axes.left + 380, y=axes.top + 240,
    )
    figure.text(72, 430, "Shell commands, compiler children and short-lived helpers live far below one second: a once-a-second sampler sees about as many of them as", size=11.5, color=MUTED)
    figure.text(72, 447, "the ratio of their life to a second. Fork, exec and exit events (the proc connector, kqueue) see all of them. That is why isotop has `Life`.", size=11.5, color=MUTED)
    out = os.path.join("docs", "media", "figures", "life-survival-sampling.svg")
    figure.save(out)
    print("wrote", out)


if __name__ == "__main__":
    main()
