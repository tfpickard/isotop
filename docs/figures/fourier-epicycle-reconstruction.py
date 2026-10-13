"""A minute of one demo process rebuilt from 1, 2, 4, 8, 12 and 16 epicycles.

Run from the repository root: python3 docs/figures/fourier-epicycle-reconstruction.py
Writes docs/media/figures/fourier-epicycle-reconstruction.svg. Standard library only.

The signal is src/ptolemy.rs's: z[n] = cpu in cores + i * log2(MiB) for 60 one-second samples of
demo process 12 (model.rs, demo_with) from t = 240 s, with the mean removed. The grey dots are the
samples; the coloured curve is the tip of a chain of the K largest DFT terms (no mean), turning
once per window, evaluated between the samples as well.
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from epicycle_w6 import SERIES, epicycles, evaluate, rms_error, window
from svgkit_w6 import AQUA, BLUE, MAGENTA, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg, heading


def main():
    z, _ = window(12, 240.0)
    ks = [1, 2, 4, 8, 12, 16]
    colors = [MAGENTA, ORANGE, YELLOW, AQUA, BLUE, VIOLET]
    svg = Svg(1080, 660, "Epicycle reconstruction of a process trajectory",
              "A 60-sample trajectory of CPU and memory reconstructed with 1, 2, 4, 8, 12 and 16 "
              "epicycles.")
    heading(svg, "One minute of demo process 12, rebuilt from K epicycles",
            "x = CPU in cores, y = memory in doublings (log2 MiB), mean removed; "
            "grey dots = the 60 samples, curve = tip of the K-term chain")
    xs = [v.real for v in z]
    ys = [v.imag for v in z]
    lim_x = (min(xs) - 0.15, max(xs) + 0.15)
    lim_y = (min(ys) - 0.15, max(ys) + 0.15)
    for index, (k, color) in enumerate(zip(ks, colors)):
        col, row = index % 3, index // 3
        x0, y0 = 64 + col * 346, 88 + row * 280
        ax = Axes(svg, x0, y0, 300, 210, lim_x, lim_y, "cores (mean removed)",
                  "doublings" if col == 0 else "", "K = %d" % k, equal=False,
                  xticks=[-0.5, 0, 0.5], yticks=[-0.5, 0, 0.5], tick_size=9.5)
        terms = epicycles(z, k)
        ax.points(xs, ys, MUTED, 2.3, opacity=0.8)
        steps = 600
        curve = [evaluate(terms, SERIES * s / steps) for s in range(steps + 1)]
        ax.line([c.real for c in curve], [c.imag for c in curve], color, 2.0)
        ax.label(lim_x[1], lim_y[1], "rms error %.3f   frequencies %s" % (
            rms_error(z, terms), ",".join(str(f) for f, _ in terms[:4]) + (",..." if k > 4 else "")),
            10, "end", TEXT, dx=-6, dy=14)
    print(svg.save("fourier-epicycle-reconstruction"))


if __name__ == "__main__":
    main()
