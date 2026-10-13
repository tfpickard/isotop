"""The Coop view's transfer functions, plotted from the constants of src/coop/sim.rs.

Run from the repository root: python3 docs/figures/coop-mappings.py
Writes docs/media/figures/coop-mappings.svg. Standard library only.

Panels: foraging speed against CPU; eggs against open files (and the cap on rotten eggs); dust
puffs against read rate; the pull towards a socket peer against measured loopback traffic and
against the CPU of the less busy end; and fox eyes against memory pressure.
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import AQUA, BLUE, MAGENTA, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg, heading


def speed(cpu):
    return 0.6 + 3.4 * cpu / (cpu + 25.0)


def clutch(files):
    return min(16, int(math.floor(math.log2(files + 1.0) + 0.5)))


def puffs(rate):
    return min(5, 1 + int(max(math.log2(rate / 65536.0), 0.0)))


def tcp_weight(rate):
    return min(math.log2(1.0 + rate / 1024.0) / 10.0, 1.5)


def together(cpu):
    return 0.8 * cpu / (cpu + 25.0)


def stepped(xs, f):
    """Left-continuous staircase points of an integer-valued f on a log grid."""
    pts = []
    last = None
    for x in xs:
        y = f(x)
        if last is not None and y != last:
            pts.append((x, last))
        pts.append((x, y))
        last = y
    return pts


def main():
    svg = Svg(1080, 700, "Coop transfer functions",
              "Six panels: foraging speed against CPU, eggs against open files, dust puffs against "
              "read rate, peer pull against traffic and co-activity, and fox eyes against memory "
              "pressure.")
    heading(svg, "The coop's transfer functions", "constants from src/coop/sim.rs and README.md; every "
            "curve saturates, so no process can dominate the yard")
    w, h, gx, gy = 286, 200, 358, 290
    ox, oy = 62, 96

    a = Axes(svg, ox, oy, w, h, (0, 400), (0, 4.4), "CPU (% of one core)", "units per second",
             "Foraging speed", yticks=[0, 1, 2, 3, 4], xticks=[0, 100, 200, 300, 400])
    cs = [c * 1.0 for c in range(0, 401)]
    a.line(cs, [speed(c) for c in cs], BLUE, 2.2)
    a.hline(4.0, MUTED, 1, "4 3")
    a.label(400, 4.0, "0.6 + 3.4 = 4.0", 10, "end", MUTED, dy=-5)
    a.label(0, 0.6, "idle floor 0.6", 10, "start", MUTED, dx=6, dy=-6)

    b = Axes(svg, ox + gx, oy, w, h, (1, 100000), (0, 17), "open files (distinct, whole flock)",
             "eggs", "Clutch: round(log2(1 + files))", xlog=True,
             yticks=[0, 4, 8, 12, 16])
    fs = [10 ** (k / 60.0) for k in range(0, 301)]
    fs = sorted(set(int(round(f)) for f in fs))
    b.polyline([(f, clutch(f)) for f in fs], ORANGE, 2.2)
    b.hline(16, MUTED, 1, "4 3")
    b.label(100000, 16, "capped at 16 from 46 340 files", 10, "end", MUTED, dy=-5)
    b.points([1, 7, 140], [clutch(1), clutch(7), clutch(140)], TEXT, 3)
    b.label(7, 3, "7 files, 3 eggs", 9.5, "start", TEXT, dx=6, dy=14)
    b.label(1, 1, "1 file, 1 egg", 9.5, "start", TEXT, dx=6, dy=14)

    c = Axes(svg, ox + 2 * gx, oy, w, h, (1024, 4194304), (0, 6), "read rate (bytes per second)",
             "dust puffs", "Dust: 1 + floor(log2(rate / 64 KiB)), at most 5", xlog=True,
             yticks=[0, 1, 2, 3, 4, 5, 6],
             xticks=[1024, 65536, 1048576, 4194304],
             xlabels=["1 KiB", "64 KiB", "1 MiB", "4 MiB"])
    rates = [2 ** (10 + k / 40.0) for k in range(0, 481)]
    pts = []
    last = None
    for r in rates:
        value = puffs(r) if r > 65536 else 0
        if last is not None and value != last:
            pts.append((r, last))
        pts.append((r, value))
        last = value
    c.polyline(pts, AQUA, 2.2)
    c.vline(65536, MUTED, 1, "4 3")
    c.label(65536, 5.6, "no puff at or below 64 KiB/s", 9.5, "start", MUTED, dx=5)

    d = Axes(svg, ox, oy + gy, w, h, (1, 1e9), (0, 1.7), "loopback TCP traffic (bytes per second)",
             "weight w", "Peer pull, measured TCP", xlog=True, yticks=[0, 0.5, 1.0, 1.5],
             xticks=[1, 1e3, 1e6, 1e9], xlabels=["1 B", "1 KB", "1 MB", "1 GB"])
    rates = [10 ** (k / 30.0) for k in range(0, 271)]
    d.line(rates, [tcp_weight(r) for r in rates], VIOLET, 2.2)
    d.hline(1.5, MUTED, 1, "4 3")
    d.label(1e9, 1.5, "cap 1.5 at 32 MiB/s", 10, "end", MUTED, dy=-5)

    e = Axes(svg, ox + gx, oy + gy, w, h, (0, 400), (0, 0.9), "CPU of the less busy end (%)",
             "weight w", "Peer pull, co-activity (Unix, unmeasured)", yticks=[0, 0.2, 0.4, 0.6, 0.8],
             xticks=[0, 100, 200, 300, 400])
    e.line(cs, [together(x) for x in cs], YELLOW, 2.2)
    e.hline(0.8, MUTED, 1, "4 3")
    e.label(400, 0.8, "0.8 at most", 10, "end", MUTED, dy=-5)

    f = Axes(svg, ox + 2 * gx, oy + gy, w, h, (0, 100), (0, 7), "memory pressure psi (%)",
             "pairs  |  units", "Fox eyes", yticks=[0, 1, 2, 3, 4, 5, 6, 7],
             xticks=[0, 20, 40, 60, 80, 100])
    ps = [p * 0.25 for p in range(0, 401)]
    f.polyline([(p, 0 if p <= 0.5 else min(6, 1 + int(p / 10))) for p in ps], MAGENTA, 2.2)
    f.line(ps, [3.0 * (1.0 - p / (p + 20.0)) + 0.4 for p in ps], ORANGE, 2.2)
    f.legend([("pairs of eyes (none up to 0.5 %)", MAGENTA, "line"),
              ("distance outside the fence", ORANGE, "line")], x=f.x + 80, y=f.y + 130, size=10.5)
    print(svg.save("coop-mappings"))


if __name__ == "__main__":
    main()
