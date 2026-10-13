"""Why hexagons, and how a frame of the hive view is filled.

Writes docs/media/figures/hive-honeycomb-frame.svg. Run from the repository root:

    python3 docs/figures/hive-honeycomb-frame.py

Left: the perimeter of a regular n-gon of unit area, P = 2 sqrt(n tan(pi / n)); of the three
regular polygons that tile the plane the hexagon needs the least wall (Hales 2001 proves that
no tiling by equal-area cells of any shape does better). Middle: one frame of 64 x 32 offset
cells filled in the arrangement of the prototype `Comb.layout` (brood in the centre, pollen
around it, royal jelly below, honey in an arch above and capped honey on its rim) from a made-up
split of 1,160 cells; the colours are mine. Right: the one cell unit of the scene, a power of
two of at least 64 KiB, as the largest frame grows from 1 MiB to 64 GiB and shrinks back, with
the hysteresis of the specification (keep the unit while the largest frame is between 700 and
2,000 cells).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

SQRT3 = math.sqrt(3.0)
KIB, MIB, GIB = 1024, 1024**2, 1024**3
COLUMNS, ROWS = 64, 32


def comb_cells(columns, rows):
    cells = []
    for row in range(rows):
        for col in range(columns - (row % 2)):
            cells.append((col + 0.5 + 0.5 * (row % 2), row * SQRT3 / 2 + 1 / SQRT3))
    width = columns
    height = (rows - 1) * SQRT3 / 2 + 2 / SQRT3
    return cells, width, height


def layout(cells, width, height, brood, pollen, jelly, honey, capped):
    centre_x, centre_y = width / 2, height * 0.58
    kinds = ["empty"] * len(cells)
    free = set(range(len(cells)))
    metric = {}
    for index, (x, y) in enumerate(cells):
        dx, dy = (x - centre_x) / 1.55, y - centre_y
        d = math.hypot(dx, dy)
        dyh = dy * 0.42 if dy < 0 else dy * 2.6
        metric[index] = (d, d - 0.9 * dy, math.sqrt(dx * dx * 0.8 + dyh * dyh))

    def take(count, which, kind):
        order = sorted(free, key=lambda i: (metric[i][which], i))[:count]
        for index in order:
            kinds[index] = kind
            free.discard(index)

    take(brood, 0, "brood")
    take(pollen, 0, "pollen")
    take(jelly, 1, "jelly")
    take(honey, 2, "honey")
    take(capped, 2, "capped")
    return kinds


COLOURS = {"empty": "#2b2116", "honey": "#e8a317", "capped": "#cdb26a", "pollen": "#d9572b",
           "jelly": "#f4f1e6", "brood": "#9db3cc"}


def next_unit(unit, largest):
    """The unit rule of the specification."""
    if (700 * unit <= largest <= 2000 * unit) or (unit == 64 * KIB and largest < 700 * unit):
        return unit
    target = 64 * KIB
    while largest > 2000 * target:
        target *= 2
    return target


def main():
    svg = P.Svg(1200, 470, "Honeycomb: cell geometry, one frame, the cell unit",
                "Perimeter of regular polygons of unit area, a frame of 2,032 offset hexagonal "
                "cells filled by kind, and the cell unit against the largest frame.")
    svg.text(24, 28, "Hive: why hexagons, the frame arrangement, the cell unit", size=16,
             weight="600")
    svg.text(24, 47, "Left: wall length per cell. Middle: brood in the centre, pollen around it, "
             "royal jelly below, honey above (convention, not measurement). Right: 1 cell = "
             "one power-of-two unit.", size=11, fill=P.MUTED)
    top, h = 100, 300

    ns = list(range(3, 13))
    perimeter = [2 * math.sqrt(n * math.tan(math.pi / n)) for n in ns]
    axes = P.Axes(svg, 60, top, 300, h, (2.5, 12.5), (3.4, 4.8), xlabel="sides n of a regular polygon",
                  ylabel="perimeter of a unit-area polygon", title="Wall per unit area",
                  xticks=[3, 4, 5, 6, 7, 8, 9, 10, 11, 12], yticks=[3.5, 4.0, 4.5])
    axes.plot(ns, perimeter, stroke=P.MUTED, width=1.2)
    for n, value in zip(ns, perimeter):
        tiles = n in (3, 4, 6)
        axes.dots([n], [value], fill=P.YELLOW if tiles else P.MUTED, radius=4.5 if tiles else 3)
    axes.hline(2 * math.sqrt(math.pi), dash="2 4")
    axes.label(12.4, 2 * math.sqrt(math.pi) - 0.05, "circle: 3.545", size=10.5, fill=P.MUTED,
               anchor="end", dy=14)
    axes.label(3, perimeter[0], "triangle 4.559", size=10.5, dx=8, dy=3)
    axes.label(4, perimeter[1], "square 4.000", size=10.5, dx=8, dy=-6)
    axes.label(6, perimeter[3], "hexagon 3.722", size=10.5, dx=8, dy=-10)

    cells, width, height = comb_cells(COLUMNS, ROWS)
    kinds = layout(cells, width, height, brood=140, pollen=260, jelly=150, honey=540, capped=70)
    left, scale = 400, 7.0
    pane_w, pane_h = width * scale + 16, height * scale + 16
    svg.rect(left, top, pane_w, pane_h, fill="#5a4630", stroke="#8a6d46", width=2.0)
    svg.text(left, top - 9, "One frame: 64 x 32 offset cells", size=13, weight="600")
    radius = 1 / SQRT3 * 0.9
    for (x, y), kind in zip(cells, kinds):
        centre_x, centre_y = left + 8 + x * scale, top + 8 + y * scale
        points = [(centre_x + radius * scale * math.cos(math.pi / 6 + k * math.pi / 3),
                   centre_y + radius * scale * math.sin(math.pi / 6 + k * math.pi / 3))
                  for k in range(6)]
        svg.polyline(points, stroke="none", width=0, fill=COLOURS[kind], close=True)
    counts = {k: kinds.count(k) for k in COLOURS}
    legend_y = top + pane_h + 22
    svg.legend(left, legend_y, [("honey %d" % counts["honey"], COLOURS["honey"], "dot"),
                                ("capped %d" % counts["capped"], COLOURS["capped"], "dot"),
                                ("pollen %d" % counts["pollen"], COLOURS["pollen"], "dot")],
               size=10.5, gap=15)
    svg.legend(left + 150, legend_y, [("royal jelly %d" % counts["jelly"], COLOURS["jelly"], "dot"),
                                      ("brood %d" % counts["brood"], COLOURS["brood"], "dot"),
                                      ("empty %d of %d" % (counts["empty"], len(cells)),
                                       "#6b5a44", "dot")], size=10.5, gap=15)

    axes3 = P.Axes(svg, 940, top, 230, h, (MIB, 64 * GIB), (100, 3000), xlog=True, ylog=True,
                   xlabel="largest frame", ylabel="cells in the largest frame",
                   title="Cell unit and hysteresis",
                   xticks=[MIB, 16 * MIB, 256 * MIB, 4 * GIB, 64 * GIB],
                   xtick_labels=["1 MiB", "16 MiB", "256 MiB", "4 GiB", "64 GiB"],
                   yticks=[100, 300, 700, 2000], ytick_labels=["100", "300", "700", "2000"])
    sizes = [MIB * 2 ** (k / 12.0) for k in range(0, 12 * 16 + 1)]
    unit = 64 * KIB
    up_x, up_y = [], []
    for size in sizes:
        unit = next_unit(unit, size)
        up_x.append(size)
        up_y.append(size / unit)
    down_x, down_y = [], []
    for size in reversed(sizes):
        unit = next_unit(unit, size)
        down_x.append(size)
        down_y.append(size / unit)
    axes3.plot(up_x, up_y, stroke=P.BLUE, width=2.0)
    axes3.plot(down_x, down_y, stroke=P.ORANGE, width=2.0, dash="6 4")
    axes3.hline(700, dash="2 4")
    axes3.hline(2000, dash="2 4")
    svg.legend(965, top + 24, [("growing", P.BLUE, "line"), ("shrinking", P.ORANGE, "dash")],
               size=10.5, gap=15)
    svg.save("docs/media/figures/hive-honeycomb-frame.svg")
    print("cells in a 64 x 32 offset comb:", len(cells), "counts", counts)
    print("perimeters", [round(v, 3) for v in perimeter])
    print("max cells growing", round(max(up_y)), "min cells once above 64 KiB-regime",
          round(min(y for x, y in zip(up_x, up_y) if x > 64 * MIB)))


if __name__ == "__main__":
    main()
