"""Boosts in the (x, w) plane and the Schlegel projection of the tesseract, as src/tesseract.rs does them.

Run from the repository root: python3 docs/figures/lorentz-schlegel-boost.py
Writes docs/media/figures/lorentz-schlegel-boost.svg. Standard library only.

Top row: the (x, w) plane (x = CPU-seconds / 30, w = time / 30 s). Left, the lab frame: worldlines
x = -1 + beta (w + 1) for beta = 0, 0.3, 0.6, 1 (on the light cone) and 1.8 (superluminal), with the
now and window-start lines. Middle, the same data boosted into the rest frame of the 0.6-core
process (tesseract.rs boost(): x' = x cosh a - w sinh a, w' = w cosh a - x sinh a about the head
event, a = artanh 0.6): that worldline is vertical, the now line tilts, the light cone does not move.
Right: hyperbolas of constant interval w^2 - x^2, which a boost slides points along.
Bottom row: the 32 edges of the tesseract with the view's projection (compress, then rotate in XW,
YW, ZW by (a, 0.10, -0.08), then divide by 3.5 - w) for XW angle a = 0, 0.12 (canonical), 45 degrees
and 90 degrees, seen through a plain isometric camera.
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import (AQUA, BLUE, MAGENTA, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg,
                       heading, rgb)

EYE, NEAREST, WORLD, COMPRESSION = 3.5, 0.75, 25.0, 0.6


def boost(point, pivot, rapidity):
    ch, sh = math.cosh(rapidity), math.sinh(rapidity)
    dx, dw = point[0] - pivot[0], point[1] - pivot[1]
    return (pivot[0] + dx * ch - dw * sh, pivot[1] + dw * ch - dx * sh)


def compress(v):
    m = abs(v)
    return v if m <= 1 else math.copysign(1 + COMPRESSION * math.tanh((m - 1) / COMPRESSION), v)


def rotation(angles):
    def plane(a, b, angle):
        m = [[1.0 if i == j else 0.0 for j in range(4)] for i in range(4)]
        s, c = math.sin(angle), math.cos(angle)
        m[a][a], m[a][b], m[b][a], m[b][b] = c, -s, s, c
        return m

    def mul(l, r):
        return [[sum(l[i][k] * r[k][j] for k in range(4)) for j in range(4)] for i in range(4)]

    return mul(mul(plane(0, 3, angles[0]), plane(1, 3, angles[1])), plane(2, 3, angles[2]))


def project(point, matrix):
    p = [compress(point[0]), point[1], point[2], compress(point[3])]
    x, y, z, w = [sum(matrix[i][k] * p[k] for k in range(4)) for i in range(4)]
    factor = 1.0 / max(EYE - w, NEAREST)
    return (x * WORLD * factor, z * WORLD * factor, y * WORLD * factor), w


def isometric(world):
    X, Y, Z = world
    tilt = math.radians(30)
    u = (X - Y) / math.sqrt(2)
    v = Z * math.cos(tilt) + (X + Y) / math.sqrt(2) * math.sin(tilt)
    return u, v


def main():
    svg = Svg(1100, 690, "Boost and Schlegel projection",
              "The (x, w) plane before and after a Lorentz boost, hyperbolas of constant "
              "interval, and the tesseract's Schlegel projection at four XW rotations.")
    heading(svg, "A boost slides along hyperbolas; a rotation turns the tesseract inside out",
            "top: (x, w) in units of 30 CPU-seconds and 30 seconds; bottom: the view's projection, "
            "violet = edges along w, orange = the cell at w = +1, blue = the cell at w = -1")
    betas = [(0.0, MUTED), (0.3, AQUA), (0.6, ORANGE), (1.0, YELLOW), (1.8, BLUE)]
    head = (-1 + 0.6 * 2, 1.0)
    a = math.atanh(0.6)
    for panel, (title, frame) in enumerate((("lab frame", False), ("boosted: rest frame of the 0.6-core process", True))):
        ax = Axes(svg, 60 + panel * 350, 86, 300, 240, (-2.2, 2.6), (-1.5, 1.5), "x (CPU-seconds / 30)",
                  "w (time / 30 s)" if panel == 0 else "", title, equal=False, title_size=11.5,
                  xticks=[-2, -1, 0, 1, 2], yticks=[-1, 0, 1])
        for sign in (1, -1):
            ends = [(-1.5, -1.5 * sign), (2.6, 2.6 * sign)]
            pts = [boost(p, head, a) if frame else p for p in ends]
            ax.polyline(pts, YELLOW, 1, dash="4 3", opacity=0.55)
        # the light cone through the head event, as the lab frame draws it: x = w + const
        for beta, color in betas:
            line = [(-1 + beta * (w + 1), w) for w in (-1.0, 1.0)]
            line = [boost(p, head, a) if frame else p for p in line]
            ax.polyline(line, color, 2.0 if beta == 0.6 else 1.5)
            if ax.xlim[0] <= line[1][0] <= ax.xlim[1] and ax.ylim[0] <= line[1][1] <= ax.ylim[1]:
                ax.svg.circle(ax.px(line[1][0]), ax.py(line[1][1]), 3, color)
        for w, label in ((1.0, "now"), (-1.0, "60 s ago")):
            pts = [(-2.2, w), (2.6, w)]
            pts = [boost(p, head, a) if frame else p for p in pts]
            ax.polyline(pts, rgb(TEXT, 0.5), 1, dash="2 3")
        if panel == 0:
            ax.label(2.55, 1.1, "now", 9.5, "end", MUTED)
            ax.label(2.55, -0.9, "60 s ago", 9.5, "end", MUTED)
            ax.legend([("beta 0 (idle)", MUTED, "line"), ("0.3", AQUA, "line"), ("0.6", ORANGE, "line"),
                       ("1 (c = one core)", YELLOW, "line"), ("1.8 (superluminal)", BLUE, "line")],
                      x=ax.x + 12, y=ax.y + 14, size=9.5)
        else:
            ax.label(2.55, -1.4, "light cone unchanged; the 0.6 worldline is vertical; 'now' tilts", 9.5, "end", MUTED)
    inv = Axes(svg, 760, 86, 290, 240, (-2.2, 2.2), (-1.5, 1.5), "x'", "w'", "constant interval w'^2 - x'^2",
               title_size=11.5, xticks=[-2, -1, 0, 1, 2], yticks=[-1, 0, 1])
    for c in (0.25, 1.0, 2.0):
        for sign, color in ((1, BLUE), (-1, BLUE)):
            xs = [k * 0.05 for k in range(-60, 61)]
            pts = [(x, sign * math.sqrt(x * x + c)) for x in xs]
            inv.polyline(pts, color, 1.4, opacity=0.8)
    for c in (0.25, 1.0, 2.0):
        for sign in (1, -1):
            ws = [k * 0.05 for k in range(-40, 41)]
            pts = [(sign * math.sqrt(w * w + c), w) for w in ws]
            inv.polyline(pts, ORANGE, 1.4, opacity=0.8)
    for sign in (1, -1):
        inv.polyline([(-2.2, -2.2 * sign), (2.2, 2.2 * sign)], YELLOW, 1.2, dash="4 3")
    inv.polyline([(math.sqrt(w * w - 0.64) * 0 + x, math.sqrt(x * x + 0.64)) for x in [k * 0.05 for k in range(-60, 61)]], MAGENTA, 1.2, dash="3 3")
    pt = (0.6, 1.0)
    for rapidity in (0.0, 0.5, 1.0):
        q = boost(pt, (0, 0), rapidity)
        inv.svg.circle(inv.px(q[0]), inv.py(q[1]), 3.2, MAGENTA)
    inv.label(2.15, 1.35, "blue: timelike, orange: spacelike", 9.5, "end", MUTED)
    inv.label(2.15, -1.38, "magenta: one event, three frames", 9.5, "end", MUTED)

    # bottom row
    angles = [0.0, 0.12, math.pi / 4, math.pi / 2]
    names = ["XW angle 0", "0.12 rad (canonical)", "45 degrees", "90 degrees"]
    corners = [[(1 if i >> k & 1 else -1) for k in range(4)] for i in range(16)]
    for index, (angle, name) in enumerate(zip(angles, names)):
        matrix = rotation([angle, 0.10, -0.08])
        box = Axes(svg, 40 + index * 262, 424, 236, 236, (-14, 14), (-14, 14), "", "", name, equal=True,
                   xticks=[], yticks=[], grid=False, title_size=11.5)
        screen = []
        for corner in corners:
            world, _ = project(corner, matrix)
            screen.append(isometric(world))
        for i in range(16):
            for axis in range(4):
                j = i | (1 << axis)
                if j == i:
                    continue
                a_, b_ = corners[i], corners[j]
                if axis == 3:
                    color = VIOLET
                elif a_[3] > 0:
                    color = ORANGE
                else:
                    color = BLUE
                (ux, uy), (vx, vy) = screen[i], screen[j]
                svg.line(box.px(ux), box.py(vy * 0 + uy), box.px(vx), box.py(vy), color, 1.5, opacity=0.9)
    print(svg.save("lorentz-schlegel-boost"))


if __name__ == "__main__":
    main()
