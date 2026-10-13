"""Building a ridgeline: lines alone, lines with hidden-line removal, and Strata's curtains.

Writes docs/media/figures/strata-ridgeline.svg. Run from the repository root:

    python3 docs/figures/strata-ridgeline.py

The data are made up (fixed seed): 16 ridges of 60 samples, each a few Gaussian pulses and a
little noise, so that the three panels draw the same numbers.

  (a) the profiles as bare polylines, stacked with a vertical offset: every line is visible
      through every other.
  (b) the same, drawn back to front, each ridge filled below its line with the background
      colour (the painter's algorithm), so a front ridge hides what lies behind it. This is the
      cover of Joy Division's "Unknown Pleasures" (1979).
  (c) Strata's construction: each ridge is a vertical curtain in the plane y = const, with the
      isotop camera's orthographic isometric projection (pitch asin(1 / sqrt 3), rotation 0:
      screen x = (x - y) / sqrt 2 * sqrt(3/2), screen up = (z cos p - (x + y) / sqrt 2 sin p) *
      sqrt(3/2) with y pointing down the page). The rasterizer's depth buffer does the hiding; a
      back-to-front order of the curtains gives the same picture here.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

ROWS, COLUMNS = 16, 60


class Lcg:
    """A fixed-seed generator, so the figure never changes."""

    def __init__(self, seed):
        self.state = seed

    def next(self):
        self.state = (self.state * 1103515245 + 12345) & 0x7FFFFFFF
        return self.state / 0x7FFFFFFF


def data():
    rng = Lcg(20240229)
    rows = []
    for row in range(ROWS):
        pulses = [(rng.next() * COLUMNS, 1.2 + 2.2 * rng.next(), 0.3 + 0.7 * rng.next())
                  for _ in range(3 + int(4 * rng.next()))]
        values = []
        for t in range(COLUMNS):
            value = sum(a * math.exp(-((t - c) ** 2) / (2 * w * w)) for c, w, a in pulses)
            values.append(min(1.0, value + 0.05 * rng.next()))
        rows.append(values)
    return rows


def main():
    rows = data()
    svg = S.Svg(1020, 410, "Building a ridgeline",
                "The same sixteen profiles drawn as bare lines, with hidden-line removal, and as "
                "curtains under the isotop camera.")
    palette = [S.SLATE, S.TEAL, S.GOLD, S.VIOLET]

    def front(left, top, filled):
        width, spacing, scale = 300.0, 14.0, 52.0
        for row in range(ROWS):
            base = top + 60 + row * spacing
            points = [(left + width * t / (COLUMNS - 1), base - scale * v)
                      for t, v in enumerate(rows[row])]
            if filled:
                svg.polygon([(points[0][0], base + 1)] + points + [(points[-1][0], base + 1)],
                            fill=S.BACKGROUND)
            svg.polyline(points, S.TEXT, 1.2, opacity=0.9)

    svg.text(20, 30, "(a) lines only", 13, weight="600")
    front(20, 30, False)
    svg.text(360, 30, "(b) painter's algorithm: back to front, filled", 13, weight="600")
    front(360, 30, True)

    # (c) Strata-style curtains through the isotop camera, with Strata's own proportions: one
    # unit of x per second of history, rows 2.4 apart, height 0.15 + 12 v.
    pitch = math.asin(1 / math.sqrt(3))

    def raw(x, y, z):
        across = (x - y) / math.sqrt(2)
        along = (x + y) / math.sqrt(2)
        return (across, -(z * math.cos(pitch) - along * math.sin(pitch)))

    drawn = []
    for row in range(ROWS):
        y = row * 2.4
        xs = [t - (COLUMNS - 1) for t in range(COLUMNS)]
        top = [raw(xs[t], y, 0.15 + 12.0 * rows[row][t]) for t in range(COLUMNS)]
        low = [raw(xs[t], y, 0.45 * (0.15 + 12.0 * rows[row][t])) for t in range(COLUMNS)]
        a, b = raw(xs[0], y, 0), raw(xs[-1], y, 0)
        drawn.append((row, a, b, top, low))
    every = [p for _, a, b, top, low in drawn for p in [a, b] + top]
    lo_x, hi_x = min(p[0] for p in every), max(p[0] for p in every)
    lo_y, hi_y = min(p[1] for p in every), max(p[1] for p in every)
    scale = min(300.0 / (hi_x - lo_x), 300.0 / (hi_y - lo_y))
    fit = lambda p: (700 + (p[0] - lo_x) * scale, 60 + (p[1] - lo_y) * scale)
    svg.text(700, 30, "(c) Strata: curtains, isotop's camera", 13, weight="600")
    for row, a, b, top, low in drawn:
        colour = palette[min(row * 4 // ROWS, 3)]
        svg.polygon([fit(a)] + [fit(p) for p in top] + [fit(b)], fill=S.tint(colour, 0.34))
        svg.polygon([fit(a)] + [fit(p) for p in low] + [fit(b)], fill="#0e1116", opacity=0.45)
        svg.polyline([fit(p) for p in top], colour, 1.3)
    svg.text(20, 384, "Panel (a) hides nothing; (b) and (c) hide what a nearer ridge covers. In (c) time "
             "runs along x (now at the lower right) and rows run from kernel (back) to containers (front).", 11, fill=S.MUTED)
    print(svg.save("strata-ridgeline"))


if __name__ == "__main__":
    main()
