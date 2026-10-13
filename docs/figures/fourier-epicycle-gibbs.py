"""Gibbs ringing in an epicycle chain: a process whose memory climbs through the whole window.

Run from the repository root: python3 docs/figures/fourier-epicycle-gibbs.py
Writes docs/media/figures/fourier-epicycle-gibbs.svg. Standard library only.

A DFT treats its 60-sample window as one period of a periodic signal. A ramp (a leak: memory in
doublings rising by 1 across the minute) therefore has a jump of about -1 where the last sample is
followed by the first. The chain made of the K largest terms rings on both sides of that jump,
overshooting by about 9 percent of its height however many terms are kept: the Gibbs
phenomenon, here seen with the same selection rule as src/ptolemy.rs.
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from epicycle_w6 import SERIES, epicycles, evaluate
from svgkit_w6 import AQUA, BLUE, MAGENTA, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg, heading


def main():
    n = SERIES
    ramp = [n_ / n for n_ in range(n)]
    mean = sum(ramp) / n
    z = [complex(0.0, v - mean) for v in ramp]
    jump = z[-1].imag - z[0].imag + (z[1].imag - z[0].imag)  # across the wrap, one sample step later
    svg = Svg(1080, 440, "Gibbs ringing of an epicycle chain at the window wrap",
              "Reconstruction of a ramp with 4, 12 and 24 epicycles, and the overshoot against "
              "the number of terms kept.")
    heading(svg, "Gibbs ringing: a memory ramp rebuilt from K epicycles",
            "z[n] = i (n / 60), mean removed; the periodic extension jumps by -1 at the wrap")
    left = Axes(svg, 64, 90, 600, 280, (-12, 72), (-0.75, 0.75), "time within the window (samples, "
                "continued periodically)", "memory, doublings (mean removed)", "Imaginary part of the chain's tip",
                xticks=[-10, 0, 10, 20, 30, 40, 50, 60, 70], yticks=[-0.5, 0, 0.5])
    left.points([m for m in range(n)], [v.imag for v in z], MUTED, 2.2)
    for k, color in ((4, YELLOW), (12, AQUA), (24, MAGENTA)):
        terms = epicycles(z, k)
        ts = [-12 + 84 * s / 900 for s in range(901)]
        left.line(ts, [evaluate(terms, t).imag for t in ts], color, 1.9)
    left.legend([("samples", MUTED, "dot"), ("K = 4", YELLOW, "line"), ("K = 12 (what isotop keeps)", AQUA, "line"),
                 ("K = 24", MAGENTA, "line")], x=left.x + 90, y=left.y + 20, size=10.5)
    left.label(60, 0.62, "wrap: last sample -> first", 10, "middle", TEXT)
    left.vline(60, MUTED, 1, "3 3")

    right = Axes(svg, 760, 90, 280, 280, (0, 29), (0, 0.16), "terms kept K",
                 "overshoot / jump", "Overshoot past the plateau",
                 xticks=[0, 6, 12, 18, 24, 29], yticks=[0, 0.05, 0.1, 0.15])
    ks, over = [], []
    for k in range(2, 30):
        terms = epicycles(z, k)
        # the largest excess of the chain over the ideal ramp on the high side of the jump
        excess = max(evaluate(terms, 40.0 + t / 40.0).imag - ((40.0 + t / 40.0) / n - mean)
                     for t in range(0, 40 * 20 - 40))
        ks.append(k)
        over.append(excess / 1.0)
    right.line(ks, over, ORANGE, 2.2)
    right.hline(0.0895, VIOLET, 1, "4 3")
    right.label(29, 0.0895, "0.0895", 10, "end", VIOLET, dy=-5)
    svg.text(24, 428, "0.0895 is (Si(pi)/pi - 1/2): the classical Gibbs overshoot as a share of the jump; "
                      "finite N and the 1/k amplitude ordering change it only slightly.", 10.5, "start", MUTED)
    print(svg.save("fourier-epicycle-gibbs"))
    print("overshoots", [round(o, 4) for o in over[:12]], "last", round(over[-1], 4))


if __name__ == "__main__":
    main()
