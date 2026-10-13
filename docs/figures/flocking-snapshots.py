"""Snapshots of a flock at three noise levels, from the two reference simulations.

Run from the repository root: python3 docs/figures/flocking-snapshots.py
Writes docs/media/figures/flocking-snapshots.svg. Standard library only; seeded.

Top row: the Coop yard rule (flock_sim_w6.yard) after 60 simulated seconds for eta = 0.5, 3.64 and
4.5 radians, which is CPU pressure 0 %, 20 % and 40 % with a steady CPU (eta = 0.5 + 5.5 psi / (psi +
15)). Bottom row: the 1995 rule on a periodic square (flock_sim_w6.vicsek_periodic) after 1000
steps, for eta = 1, 4 and 5.5. Each arrow is a body, pointing along its heading, coloured by
heading angle so that an aligned flock is one colour.
"""

import colorsys
import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from flock_sim_w6 import noise, vicsek_periodic, yard
from svgkit_w6 import Axes, Svg, heading, TEXT, MUTED

TAU = 2 * math.pi


def hue(angle):
    r, g, b = colorsys.hsv_to_rgb((angle % TAU) / TAU, 0.75, 0.95)
    return "#%02x%02x%02x" % (int(r * 255), int(g * 255), int(b * 255))


def main():
    svg = Svg(1080, 700, "Snapshots of flocks at three noise levels",
              "Positions and headings of 60 chickens in the Coop yard rule and of 256 particles "
              "in the periodic Vicsek model at low, intermediate and high heading noise.")
    heading(svg, "What the order parameter looks like", "arrow = one body along its heading, colour = "
            "heading angle (an aligned flock is a single colour)")
    cases = [("yard", noise(0.0)), ("yard", noise(20.0)), ("yard", noise(40.0)),
             ("box", 1.0), ("box", 4.0), ("box", 5.5)]
    for index, (kind, eta) in enumerate(cases):
        col, row = index % 3, index // 3
        x0, y0 = 40 + col * 346, 84 + row * 300
        if kind == "yard":
            order, pos, theta = yard(eta, seed=11, state=True)
            xs = [p[0] for p in pos]
            ys = [p[1] for p in pos]
            span = max(max(xs) - min(xs), max(ys) - min(ys), 20.0) * 0.5 + 3
            cx, cy = (max(xs) + min(xs)) / 2, (max(ys) + min(ys)) / 2
            lim = (cx - span, cx + span), (cy - span, cy + span)
            length = 0.09 * span * 2
            title = "yard, eta = %.2f: phi = %.2f" % (eta, order)
        else:
            order, pos, theta = vicsek_periodic(256, 6.4, eta, 1000, 500, seed=3, state=True)
            lim = (0, 6.4), (0, 6.4)
            length = 0.22
            title = "periodic box, eta = %.1f: phi = %.2f" % (eta, order)
        ax = Axes(svg, x0, y0, 300, 250, lim[0], lim[1], "", "", title, equal=True, xticks=[],
                  yticks=[], grid=False, title_size=11.5)
        for (px, py), a in zip(pos, theta):
            x1, y1 = px + length * math.cos(a), py + length * math.sin(a)
            svg.line(ax.px(px), ax.py(py), ax.px(x1), ax.py(y1), hue(a), 1.8)
            svg.circle(ax.px(px), ax.py(py), 1.6, hue(a))
    print(svg.save("flocking-snapshots"))


if __name__ == "__main__":
    main()
