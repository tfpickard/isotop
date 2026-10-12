"""Retrograde motion from a deferent and one epicycle, and when it happens.

Run from the repository root: python3 docs/figures/fourier-epicycle-retrograde.py
Writes docs/media/figures/fourier-epicycle-retrograde.svg. Standard library only.

A planet at p(t) = R e^{i w_d t} + r e^{i w_e t} seen from Earth at the origin. Its apparent
longitude falls (retrograde) exactly where Im(conj(p) p') < 0, the test in src/ptolemy.rs. With
r < R the minimum of Im(conj(p) p') over the phase between the two circles is
(R - r)(R w_d - r w_e), so retrograde motion exists if and only if r w_e > R w_d: the epicycle's tip
speed beats the deferent's. The four panels have r w_e / (R w_d) = 0.5, 1 (a stationary point,
a cusp), 1.5 and 3. The last panel is a Mars-like ratio: r / R = 39.5 / 60 (the Almagest value as
tabulated, unverified here) and w_e / w_d = 686.98 d / 365.24 d.
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import BLUE, MUTED, ORANGE, RED, TEXT, YELLOW, Axes, Svg, heading

TAU = 2 * math.pi


def path(radius_ratio, speed_ratio, turns):
    """Points p(t), velocities and the retrograde flag for R = 1, w_d = 1."""
    steps = 3000
    out = []
    for s in range(steps + 1):
        t = turns * TAU * s / steps
        p = complex(math.cos(t), math.sin(t)) + radius_ratio * complex(
            math.cos(speed_ratio * t), math.sin(speed_ratio * t))
        v = 1j * complex(math.cos(t), math.sin(t)) + radius_ratio * 1j * speed_ratio * complex(
            math.cos(speed_ratio * t), math.sin(speed_ratio * t))
        out.append((p, (p.conjugate() * v).imag < 0))
    return out


def main():
    cases = [("r w_e / R w_d = 0.5: never retrograde", 0.5, 1.0, 1),
             ("= 1: stationary points, cusps", 0.5, 2.0, 1),
             ("= 1.5: loops", 0.5, 3.0, 1),
             ("Mars-like: r/R = 0.658, w_e/w_d = 1.88", 39.5 / 60.0, 686.98 / 365.2422, 1)]
    svg = Svg(1080, 330, "Retrograde loops from a deferent and an epicycle",
              "Apparent paths of a planet on a deferent circle and an epicycle for four ratios of "
              "epicycle tip speed to deferent speed; retrograde stretches in red.")
    heading(svg, "Retrograde motion: the epicycle outruns the deferent",
            "Earth at the centre, deferent R = 1 (grey); red where the apparent longitude falls")
    for index, (title, ratio, speed, _) in enumerate(cases):
        x0 = 36 + index * 262
        ax = Axes(svg, x0, 82, 236, 210, (-1.8, 1.8), (-1.8, 1.8), "", "", title, equal=True,
                  xticks=[], yticks=[], grid=False, title_size=10.5)
        svg.circle(ax.px(0), ax.py(0), (ax.px(1) - ax.px(0)), "none", "#30363d", 1)
        svg.circle(ax.px(0), ax.py(0), 3.2, "#26b88a")
        turns = 1 if index < 3 else 1
        # follow the curve over enough revolutions to close it
        if index == 0:
            turns = 1
        elif index == 3:
            turns = 1
        points = path(ratio, speed, turns)
        if index == 3:
            # Mars-like ratio does not close in one deferent turn; follow 15 synodic cycles
            # worth of epicycle turns so the loop pattern repeats around the sky
            points = path(ratio, speed, 7)
        retro = 0
        for (a, back_a), (b, _) in zip(points, points[1:]):
            colour = RED if back_a else BLUE
            retro += 1 if back_a else 0
            ax.polyline([(a.real, a.imag), (b.real, b.imag)], colour, 1.6 if back_a else 1.2, opacity=0.9)
        share = retro / (len(points) - 1)
        ax.label(1.75, -1.62, "retrograde %.1f%% of the time" % (100 * share), 9.5, "end", MUTED)
        if index == 3:
            print("Mars-like retrograde share %.4f  = %.1f days of a 779.9-day synodic cycle"
                  % (share, share * 779.9))
    print(svg.save("fourier-epicycle-retrograde"))


if __name__ == "__main__":
    main()
