"""The transforms the chamber view is specified to apply (issue #42), as curves.

Writes docs/media/figures/chamber-mapping.svg. Run from the repository root:

    python3 docs/figures/chamber-mapping.py

Constants from the issue: window radius R = 40 world units, K_R = 10.5 units per core, radius
clamp [0.4, 0.45 R], visual speed of light 6 units/s, bubble density 3.8 per unit at beta = 1
with 1 / beta^2 clamped at 8, mass 1.2 per GiB, neutral below 0.10 cores.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

WINDOW = 40.0
K_RADIUS = 10.5
RADIUS_LOW, RADIUS_HIGH = 0.4, 0.45 * WINDOW
C_VISUAL = 6.0
DENSITY = 3.8
CLAMP = 8.0
MASS_PER_GIB = 1.2
NEUTRAL = 0.10


def radius(cpu):
    return min(max(K_RADIUS * cpu, RADIUS_LOW), RADIUS_HIGH)


def beta(cpu, gib):
    mass = MASS_PER_GIB * gib
    return cpu / math.sqrt(cpu * cpu + mass * mass)


def bubbles(cpu, gib):
    return DENSITY * min(1.0 / beta(cpu, gib) ** 2, CLAMP)


def main():
    svg = P.Svg(1230, 420, "Chamber mapping curves",
                "Larmor radius, speed and bubble density of a track against the CPU of its "
                "process, for three memory sizes.")
    svg.text(24, 28, "Chamber: from CPU and memory to radius, speed and bubble density",
             size=16, weight="600")
    svg.text(24, 47, "p = CPU in cores, m = 1.2 per GiB of memory; tracks exist only above "
             "0.10 cores (grey band).", size=11, fill=P.MUTED)
    top, h, w = 100, 260, 320
    cpus = [k * 0.01 for k in range(1, 401)]

    axes = P.Axes(svg, 70, top, w, h, (0, 4), (0, 20), xlabel="CPU, cores",
                  ylabel="Larmor radius r (world units)", title="r = K_R p, clamped",
                  xticks=[0, 1, 2, 3, 4], yticks=[0, 5, 10, 15, 20])
    axes.plot(cpus, [radius(c) for c in cpus], stroke=P.YELLOW, width=2.2)
    axes.hline(RADIUS_HIGH, dash="2 4")
    axes.label(4, RADIUS_HIGH + 0.9, "0.45 R = 18", size=10.5, fill=P.MUTED, anchor="end")
    axes.label(1.72, 10.5 * 1.72, "r = 10.5 p", size=10.5, fill=P.YELLOW, dx=8, dy=14)
    svg.rect(axes.px(0), top, axes.px(NEUTRAL) - axes.px(0), h, fill=P.MUTED, opacity=0.25)

    axes2 = P.Axes(svg, 470, top, w, h, (0, 4), (0, 1.0), xlabel="CPU, cores",
                   ylabel="beta = p / sqrt(p^2 + m^2)", title="Speed along the track",
                   xticks=[0, 1, 2, 3, 4], yticks=[0, 0.25, 0.5, 0.75, 1.0])
    for gib, colour in ((0.1, P.AQUA), (1.0, P.BLUE), (4.0, P.ORANGE)):
        axes2.plot(cpus, [beta(c, gib) for c in cpus], stroke=colour, width=2.2)
    svg.rect(axes2.px(0), top, axes2.px(NEUTRAL) - axes2.px(0), h, fill=P.MUTED, opacity=0.25)
    svg.legend(560, top + h - 52, [("0.1 GiB", P.AQUA, "line"), ("1 GiB", P.BLUE, "line"),
                                   ("4 GiB", P.ORANGE, "line")], size=10.5, gap=15)

    axes3 = P.Axes(svg, 880, top, w, h, (0, 4), (0, 32), xlabel="CPU, cores",
                   ylabel="bubbles per world unit", title="3.8 min(1 / beta^2, 8)",
                   xticks=[0, 1, 2, 3, 4], yticks=[0, 8, 16, 24, 32])
    for gib, colour in ((0.1, P.AQUA), (1.0, P.BLUE), (4.0, P.ORANGE)):
        axes3.plot(cpus, [bubbles(c, gib) for c in cpus], stroke=colour, width=2.2)
    axes3.hline(DENSITY * CLAMP, dash="2 4")
    axes3.label(4, DENSITY * CLAMP + 1.4, "clamp 30.4", size=10.5, fill=P.MUTED, anchor="end")
    svg.rect(axes3.px(0), top, axes3.px(NEUTRAL) - axes3.px(0), h, fill=P.MUTED, opacity=0.25)
    svg.save("docs/media/figures/chamber-mapping.svg")
    for gib in (0.1, 1.0, 4.0):
        edge = MASS_PER_GIB * gib / math.sqrt(CLAMP - 1.0)
        print("memory", gib, "GiB: density clamp reached below CPU", round(edge, 3), "cores")
    print("radius clamp reached at", round(RADIUS_HIGH / K_RADIUS, 3), "cores; low clamp at",
          round(RADIUS_LOW / K_RADIUS, 4), "cores")


if __name__ == "__main__":
    main()
