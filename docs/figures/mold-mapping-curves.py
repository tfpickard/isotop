"""The transforms the mold view is specified to apply (issue #41), as curves.

Writes docs/media/figures/mold-mapping-curves.svg. Run from the repository root:

    python3 docs/figures/mold-mapping-curves.py

Left: corridor amplitude against link traffic, 0.55 min(1, 0.04 min(sockets, 4) +
log10(1 + bytes) / 7). Middle: oat-flake radius in grid cells against memory, from the area
(70 + 0.24 MiB) (384 / 400)^2 cells clamped to a radius of 2.5 to 16. Right: the scent of a flake
at distance d from its centre in flake radii, exp(-2.5 (d - 1)) (0.15 + 0.012 cpu).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402


def corridor(bytes_per_second, sockets):
    return 0.55 * min(1.0, 0.04 * min(sockets, 4) + math.log10(1.0 + bytes_per_second) / 7.0)


def flake_radius(mebibytes):
    area = (70.0 + 0.24 * mebibytes) * (384.0 / 400.0) ** 2
    return min(16.0, max(2.5, math.sqrt(area / math.pi)))


def scent(distance, cpu):
    return math.exp(-2.5 * max(distance - 1.0, 0.0)) * (0.15 + 0.012 * cpu)


def main():
    svg = P.Svg(1180, 400, "Mold mapping curves",
                "Corridor amplitude against link traffic, flake radius against memory and "
                "scent against distance for several CPU loads.")
    svg.text(24, 28, "Mold: three of the specified transforms", size=16, weight="600")
    top, w, h = 90, 300, 240

    xs = [10 ** (k / 20.0) for k in range(0, 161)]
    axes = P.Axes(svg, 70, top, w, h, (1, 1e8), (0, 0.6), xlog=True,
                  xlabel="link traffic, bytes per second", ylabel="corridor amplitude",
                  title="Corridor laid along a link", yticks=[0, 0.2, 0.4, 0.6],
                  xticks=[1, 1e2, 1e4, 1e6, 1e8])
    for sockets, colour in ((0, P.MUTED), (1, P.BLUE), (2, P.AQUA), (4, P.ORANGE)):
        axes.plot(xs, [corridor(x, sockets) for x in xs], stroke=colour, width=2.0)
    axes.hline(0.55, dash="2 4")
    axes.label(2, 0.575, "cap 0.55", size=10.5, fill=P.MUTED)
    svg.legend(80, top + 18, [("0 sockets (bytes only)", P.MUTED, "line"),
                              ("1 socket", P.BLUE, "line"), ("2 sockets", P.AQUA, "line"),
                              ("4 or more", P.ORANGE, "line")], size=10.5, gap=15)

    mib = [10 ** (k / 20.0) for k in range(0, 101)]
    axes = P.Axes(svg, 450, top, w, h, (1, 1e5), (0, 18), xlog=True,
                  xlabel="process memory, MiB", ylabel="flake radius, grid cells",
                  title="Oat flake size", yticks=[0, 4, 8, 12, 16],
                  xticks=[1, 10, 100, 1e3, 1e4, 1e5],
                  xtick_labels=["1", "10", "100", "1 GiB", "10 GiB", "100 GiB"])
    axes.plot(mib, [flake_radius(m) for m in mib], stroke=P.YELLOW, width=2.2)
    axes.hline(16, dash="2 4")
    axes.label(1.5, 16.6, "clamp 16 above %.0f MiB" % ((16.0**2 * math.pi / 0.9216 - 70) / 0.24),
               size=10.5, fill=P.MUTED)
    axes.hline(flake_radius(0), dash="2 4")
    axes.label(1.5, flake_radius(0) - 1.4, "0 MiB: %.1f cells (the clamp of 2.5 never binds)" %
               flake_radius(0), size=10.5, fill=P.MUTED)

    distance = [k / 40.0 for k in range(0, 161)]
    axes = P.Axes(svg, 830, top, w, h, (0, 4), (0, 1.5), xlabel="distance from centre, flake radii",
                  ylabel="attractant added per step", title="Scent halo",
                  xticks=[0, 1, 2, 3, 4], yticks=[0, 0.5, 1.0, 1.5])
    for cpu, colour in ((0, P.MUTED), (10, P.BLUE), (50, P.AQUA), (100, P.YELLOW),
                        (100, None), (140, P.ORANGE)):
        if colour is None:
            continue
        axes.plot(distance, [scent(d, cpu) for d in distance], stroke=colour, width=2.0)
    svg.legend(1010, top + 18, [("cpu 0%", P.MUTED, "line"), ("10%", P.BLUE, "line"),
                                ("50%", P.AQUA, "line"), ("100%", P.YELLOW, "line"),
                                ("140%", P.ORANGE, "line")], size=10.5, gap=15)
    axes.vline(1.0, dash="2 4")
    svg.save("docs/media/figures/mold-mapping-curves.svg")
    print("corridor at 10 MB/s, 0 sockets", round(corridor(1e7, 0), 4),
          "1 socket", round(corridor(1e7, 1), 4))
    print("flake radius at 0 MiB", round(flake_radius(0), 3), "at 1024 MiB",
          round(flake_radius(1024), 3), "clamp above MiB", (16.0**2 * math.pi / 0.9216 - 70) / 0.24)


if __name__ == "__main__":
    main()
