"""The shortest round trip a great-circle arc allows.

Writes docs/media/figures/globe-rtt.svg. Run from the repository root:

    python3 docs/figures/globe-rtt.py

For two points a great-circle distance d apart, light needs at least 2 d / v for the round trip,
with v = c in vacuum and v = c / n_g in glass of group index n_g. Single-mode fibre has n_g of
about 1.47, so a signal covers about 204 000 km/s and each 1000 km of great circle costs at least
9.8 ms of round trip. The globe's inspector prints the kernel's smoothed round-trip time
(`tcpi_rtt`) next to the arc; this is the floor under it. Real paths are longer than the great
circle (cables follow coasts and sea floors, routers add queueing and processing), so the printed
value sits above the floor; a value below it means the other end is not where the database says,
for example because the address is anycast or a CDN node.

Distances use the haversine formula with R = 6371 km:
    a = sin^2(dphi / 2) + cos phi1 cos phi2 sin^2(dlambda / 2),  d = 2 R asin(sqrt a).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

C = 299792.458  # km/s, exact
N_GROUP = 1.47
R_EARTH = 6371.0
HOME = (52.37, 4.9)
CITIES = [("New York", 40.71, -74.01), ("Sao Paulo", -23.55, -46.63),
          ("Johannesburg", -26.20, 28.05), ("Singapore", 1.35, 103.82),
          ("Tokyo", 35.68, 139.69), ("Sydney", -33.87, 151.21),
          ("San Francisco", 37.77, -122.42)]


def haversine(a, b):
    p1, p2 = math.radians(a[0]), math.radians(b[0])
    dp, dl = p2 - p1, math.radians(b[1] - a[1])
    h = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
    return 2 * R_EARTH * math.asin(math.sqrt(h))


def floor_ms(distance, index):
    return 2 * distance * index / C * 1000.0


def main():
    svg = S.Svg(900, 520, "Round-trip floor against great-circle distance",
                "Minimum round-trip time in fibre and in vacuum, with cities measured from "
                "Amsterdam.")
    ax = S.Axes(svg, 90, 60, 760, 360, (0, 20100), (0, 220))
    ax.frame(xticks=[0, 5000, 10000, 15000, 20000], yticks=[0, 50, 100, 150, 200],
             xlabel="great-circle distance from Amsterdam (km)", ylabel="minimum round trip (ms)")
    svg.text(90, 46, "RTT floor = 2 d n_g / c  (n_g = 1.47 in fibre, 1 in vacuum)", 13, weight="600")
    xs = [0, 20100]
    ax.curve(xs, [floor_ms(x, N_GROUP) for x in xs], S.GOLD, 2.4)
    ax.curve(xs, [floor_ms(x, 1.0) for x in xs], S.TEAL, 2.0, dash="5 3")
    svg.text(ax.px(14500), ax.py(floor_ms(14500, N_GROUP)) - 10, "fibre: 9.8 ms per 1000 km", 12,
             fill=S.GOLD)
    svg.text(ax.px(16200), ax.py(floor_ms(16200, 1.0)) + 24, "vacuum: 6.7 ms per 1000 km", 12,
             fill=S.TEAL)
    rows = []
    for name, lat, lon in CITIES:
        d = haversine(HOME, (lat, lon))
        rows.append((name, d, floor_ms(d, N_GROUP), floor_ms(d, 1.0)))
    for name, d, fibre, vacuum in sorted(rows, key=lambda r: r[1]):
        svg.circle(ax.px(d), ax.py(fibre), 4, S.GOLD, "#0e1116", 1)
        svg.text(ax.px(d) - 8, ax.py(fibre) - 8, "%s %d km, %.0f ms" % (name, round(d, -1), fibre),
                 10.5, "end", S.TEXT)
    svg.text(90, 470, "Half the earth's circumference, 20 015 km, costs at least 196 ms in fibre. Real "
             "paths are longer than the great circle, so a measured RTT sits above its floor.", 11,
             fill=S.MUTED)
    svg.text(90, 488, "The arcs the globe draws are great circles; their length is the lower bound on "
             "the path, not a measurement of it.", 11, fill=S.MUTED)
    print(svg.save("globe-rtt"))
    for row in sorted(rows, key=lambda r: r[1]):
        print("%-14s %7.0f km fibre %6.1f ms vacuum %6.1f ms" % row)
    print("slope fibre ms/1000km", floor_ms(1000, N_GROUP), "vacuum", floor_ms(1000, 1.0))
    print("half circumference", math.pi * R_EARTH, floor_ms(math.pi * R_EARTH, N_GROUP))


if __name__ == "__main__":
    main()
