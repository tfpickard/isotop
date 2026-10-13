"""Writes docs/media/figures/layout-vogel.svg: Vogel spirals at four divergence angles (the
golden angle against three wrong ones), and the sequence discrepancy N*D*_N of the angles {n a}.

Run from the repository root: python3 docs/figures/layout-vogel.py
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w3 import *  # noqa: E402,F403

PHI = (1 + math.sqrt(5)) / 2
SLOTS = 600
PANELS = [("golden angle  137.508°", 1 - 1 / PHI), ("137.0°", 137.0 / 360), ("138.0°", 138.0 / 360), ("120.0° (a third of a turn)", 1 / 3)]


def colour(index):
    t = index / (SLOTS - 1)
    stops = [(0.0, (63, 81, 181)), (0.5, (57, 197, 187)), (1.0, (250, 214, 60))]
    for (t0, c0), (t1, c1) in zip(stops, stops[1:]):
        if t <= t1:
            f = (t - t0) / (t1 - t0)
            return "#%02x%02x%02x" % tuple(round(a + (b - a) * f) for a, b in zip(c0, c1))
    return "#fad63c"


def discrepancy(alpha, n):
    xs = sorted((k * alpha) % 1.0 for k in range(n))
    return 1 / (2 * n) + max(abs(x - (2 * i + 1) / (2 * n)) for i, x in enumerate(xs))


def main():
    svg = Svg(1200, 690, "Vogel spirals and angle discrepancy",
              "600 slots of the golden-angle spiral against 137.0, 138.0 and 120 degrees, dotted by arrival order, "
              "and the 1-D discrepancy of the angle sequence for N = 377 against the divergence angle.")
    c = 126.0 / math.sqrt(SLOTS)
    for p, (label, alpha) in enumerate(PANELS):
        cx, cy = 150 + p * 300, 172
        svg.text(cx, 20, label, 13, INK, "middle", weight="600")
        svg.circle(cx, cy, 138, "none", GRID, 1)
        for k in range(SLOTS):
            r = c * math.sqrt(k + 0.5)
            a = 2 * math.pi * alpha * k
            svg.circle(cx + r * math.cos(a), cy + r * math.sin(a), 2.5, colour(k))
    svg.text(600, 336, "slot order: dark = early, yellow = late.  A fixed angle per panel, spacing c, offset 0.5, 600 slots.", 11, MUTED, "middle")

    n = 377
    ax = Axes(svg, 70, 376, 1090, 220, (0.0, 0.5), (0.5, 600.0), False, True)
    steps = 3600
    xs = [0.5 * (i + 1) / steps for i in range(steps)]
    ys = [max(0.5, n * discrepancy(a, n)) for a in xs]
    ax.frame([0.0, 0.1, 0.2, 0.3, 0.4, 0.5], [1, 3, 10, 30, 100, 300],
             "divergence angle as a fraction of a turn, α  (the sequence is {nα}, n < 377)",
             "N · D*  (log)", lambda v: "%g" % v, lambda v: "%g" % v)
    ax.path(xs, ys, BLUE, 1.0)
    marks = [(1 - 1 / PHI, "golden 0.382", AQUA), (1 / 3, "1/3", RED), (math.sqrt(2) - 1, "√2−1", YELLOW), (0.25, "1/4", RED), (0.2, "1/5", RED), (0.5, "1/2", RED)]
    for a, name, col in marks:
        v = max(0.5, n * discrepancy(a - 1e-4 if name == "1/2" else a + 1e-4 if name.startswith("1/") else a, n))
        svg.line(ax.px(a), ax.y, ax.px(a), ax.y + ax.h, col, 1, "3 3", 0.8)
        svg.circle(ax.px(a), ax.py(v), 3.5, col)
        svg.text(ax.px(a) + (6 if a < 0.45 else -6), ax.y + 14 + (0 if name.startswith("golden") else 14 if name.startswith("\u221a") else 0), name, 11, col, "start" if a < 0.45 else "end")
    svg.text(70, 660, "Golden angle: N·D* = 1.00 at Fibonacci N (144, 377), 1.44 at 89, 233.  Rational angles tower to ~N/q: the rays of the 120° panel.", 11, MUTED)
    svg.text(70, 678, "D* = 1/(2N) + max |x(i) − (2i−1)/(2N)| for the sorted sample; 1/N is the least any set containing 0 can reach.", 11, MUTED)
    svg.save("docs/media/figures/layout-vogel.svg")


main()
