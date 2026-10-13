#!/usr/bin/env python3
"""Frequency response of isotop's CPU pipeline: interval average, then the exponential filter.

Run from the repository root: python3 docs/figures/model-smoothing-response.py
Writes docs/media/figures/model-smoothing-response.svg.

From a continuous load x(t) to the displayed sequence at dt = 1 s and tau = 1.5 s:
  interval average (counter delta / dt):  |sinc(f dt)|, sinc(x) = sin(pi x) / (pi x)
  exponential filter, as a difference equation y[k] = a y[k-1] + (1 - a) x[k], a = exp(-dt / tau):
      |H| = (1 - a) / sqrt(1 - 2 a cos(2 pi f dt) + a^2)
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sampling_plot import BLUE, GREEN, INK, MUTED, ORANGE, PURPLE, Figure  # noqa: E402

TAU = 1.5
DT = 1.0
FLOOR = -70.0


def sinc(x):
    return 1.0 if x == 0 else math.sin(math.pi * x) / (math.pi * x)


def box(f):
    return abs(sinc(f * DT))


def ema(f):
    a = math.exp(-DT / TAU)
    w = 2 * math.pi * f * DT
    return (1 - a) / math.sqrt(1 - 2 * a * math.cos(w) + a * a)


def rc(f):
    return 1 / math.sqrt(1 + (2 * math.pi * f * TAU) ** 2)


def db(gain):
    return max(FLOOR, 20 * math.log10(max(gain, 1e-12)))


def corner():
    a = math.exp(-DT / TAU)
    return math.acos((-1 + 4 * a - a * a) / (2 * a)) / (2 * math.pi * DT)


def main():
    figure = Figure(
        900,
        470,
        "Frequency response of the CPU pipeline",
        "Gain in decibels of the interval average, the exponential filter and their product, at one sample per second.",
    )
    figure.text(60, 28, "What the displayed CPU passes (dt = 1 s, tau = 1.5 s)", size=15, weight="600")
    left = figure.axes(
        72, 62, 380, 320, (0.01, 0.5), (-30, 3), xlog=True, xlabel="load frequency (Hz)",
        ylabel="gain (dB)", title="below the Nyquist limit", yticks=[0, -3, -10, -20, -30],
        xticks=[0.01, 0.02, 0.05, 0.1, 0.2, 0.5], xformat=lambda v: f"{v:g}",
    )
    fs = [10 ** (math.log10(0.01) + i * (math.log10(0.5) - math.log10(0.01)) / 300) for i in range(301)]
    left.hline(-3, MUTED, "3 5")
    left.path(fs, [db(rc(f)) for f in fs], INK, 1.2, "6 4", 0.8)
    left.path(fs, [db(box(f)) for f in fs], ORANGE, 1.6)
    left.path(fs, [db(ema(f)) for f in fs], BLUE, 1.6)
    left.path(fs, [db(box(f) * ema(f)) for f in fs], GREEN, 2.2)
    fc = corner()
    left.vline(fc, PURPLE, "2 4", 1.1)
    left.note(fc, -26, f"-3 dB at {fc:.3f} Hz", color=PURPLE, dx=6)
    left.note(fc, -22.5, f"(RC: {1 / (2 * math.pi * TAU):.3f} Hz)", color=MUTED, dx=6)
    left.legend(
        [(ORANGE, "interval average", None), (BLUE, "exponential filter", None),
         (GREEN, "both (what is drawn)", None), (INK, "ideal RC, tau = 1.5 s", "6 4")],
        x=left.left + 12, y=left.top + 292 - 62,
    )

    right = figure.axes(
        520, 62, 340, 320, (0, 3), (FLOOR, 3), xlabel="load frequency (Hz)", ylabel="gain (dB)",
        title="and beyond it: folded, not removed", yticks=[0, -10, -20, -30, -40, -50, -60, -70],
        xticks=[0, 0.5, 1, 1.5, 2, 2.5, 3], xformat=lambda v: f"{v:g}",
    )
    fr = [i * 3.0 / 1200 for i in range(1201)]
    right.path(fr, [db(box(f) * ema(f)) for f in fr], GREEN, 2.0)
    right.vline(0.5, MUTED, "4 4")
    right.note(0.5, 1, "Nyquist, 0.5 Hz", color=MUTED, dx=5, dy=1)
    right.points([0.5], [db(box(0.5) * ema(0.5))], PURPLE, 3.6)
    right.note(0.5, db(box(0.5) * ema(0.5)), f"{db(box(0.5) * ema(0.5)):.1f} dB", color=PURPLE, dx=8, dy=-4)
    right.points([1.1], [db(box(1.1) * ema(1.1))], PURPLE, 3.6)
    right.note(1.1, db(box(1.1) * ema(1.1)), f"1.1 Hz: {db(box(1.1) * ema(1.1)):.1f} dB, seen at 0.1 Hz", color=PURPLE, dx=8, dy=-7)
    right.note(1.0, -66, "null at 1 Hz", color=MUTED, dx=4)
    right.note(2.0, -66, "null at 2 Hz", color=MUTED, dx=4)
    figure.text(
        72, 436, "The interval average is the only anti-alias filter: its gain at the Nyquist limit is 2/pi (-3.9 dB) and its first sidelobe still passes 22% at 1.4 Hz.",
        size=11.5, color=MUTED,
    )
    figure.text(
        72, 453, "A load faster than 0.5 Hz is attenuated but then reads as a slower one; only an exact multiple of the sampling frequency vanishes.",
        size=11.5, color=MUTED,
    )
    out = os.path.join("docs", "media", "figures", "model-smoothing-response.svg")
    figure.save(out)
    print("wrote", out)


if __name__ == "__main__":
    main()
