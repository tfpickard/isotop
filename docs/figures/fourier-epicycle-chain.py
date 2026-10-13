"""An epicycle chain at one instant: the K circles whose tip draws a minute of a process.

Run from the repository root: python3 docs/figures/fourier-epicycle-chain.py
Writes docs/media/figures/fourier-epicycle-chain.svg. Standard library only.

The chain is drawn as src/ptolemy.rs draws it: the largest term first, each circle centred on the
tip of the one before, a radius of |c_k|, turning at k turns per window (negative k clockwise).
Shown at t = 20 s of the window for K = 2, 4 and 12, over the curve the tip traces.
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from epicycle_w6 import SERIES, epicycles, evaluate, window
from svgkit_w6 import AQUA, BLUE, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg, heading

TAU = 2 * math.pi


def main():
    z, _ = window(12, 240.0)
    svg = Svg(1080, 330, "Epicycle chains at one instant",
              "The circles of a 2, 4 and 12 term epicycle chain at t = 20 s of the window, with "
              "the curve the tip traces.")
    heading(svg, "The machine: K circles, each riding the tip of the one before",
            "same signal as the reconstruction figure; largest term first; k turns per window, "
            "k < 0 clockwise")
    for index, k in enumerate((2, 4, 12)):
        ax = Axes(svg, 40 + index * 352, 82, 320, 222, (-0.55, 0.75), (-0.6, 0.6), "", "",
                  "K = %d" % k, equal=True, xticks=[], yticks=[], grid=False)
        terms = sorted(epicycles(z, k), key=lambda t: -abs(t[1]))
        curve = [evaluate(terms, SERIES * s / 600) for s in range(601)]
        ax.polyline([(c.real, c.imag) for c in curve], BLUE, 1.2, opacity=0.55)
        joint = 0j
        t = 20.0
        for rank, (f, c) in enumerate(terms):
            tip = joint + c * complex(math.cos(TAU * f * t / SERIES), math.sin(TAU * f * t / SERIES))
            radius = abs(c) * (ax.px(1) - ax.px(0))
            if radius > 1.0:
                svg.circle(ax.px(joint.real), ax.py(joint.imag), radius, "none", "#6e7681", 1)
            svg.line(ax.px(joint.real), ax.py(joint.imag), ax.px(tip.real), ax.py(tip.imag),
                     "#c9d1d9", 1.3)
            joint = tip
        svg.circle(ax.px(joint.real), ax.py(joint.imag), 4, ORANGE)
        svg.circle(ax.px(0), ax.py(0), 2.5, MUTED)
        ax.label(0.74, -0.57, "tip = planet; origin = the mean", 9.5, "end", MUTED)
    print(svg.save("fourier-epicycle-chain"))


if __name__ == "__main__":
    main()
