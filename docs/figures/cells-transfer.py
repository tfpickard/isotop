"""The transfer functions of a Cells cell, from src/cells.rs.

Writes docs/media/figures/cells-transfer.svg. Run from the repository root:

    python3 docs/figures/cells-transfer.py

  (a) radius = max(1.3 + 0.16 sqrt(MiB), 0.75 sqrt(span + 1) + 1) world units, so the cell's area
      is proportional to memory once memory is large: area scaling, not volume scaling.
  (b) the dashed memory-limit ring is drawn at radius r sqrt(1 / used), clamped to 1.04 r .. 1.6 r,
      and only once used >= 0.25 of the limit, so ring and cell areas stand in the ratio of limit
      to use wherever the clamp does not bite. Its colour runs from amber to red between 60 % and
      100 % of the limit.
  (c) the membrane's wobble amplitude is 0.03 + 0.22 p / (p + 15) of the radius, p being the
      largest of the cgroup's cpu, memory and io `some avg10` percentages.
  (d) the CPU arc sweeps min(cpu / quota, 1) of a turn, where quota is cpu.max as a percentage of
      one core and 100 without one; it turns red above 90 % of a quota.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402


def main():
    svg = S.Svg(980, 700, "Transfer functions of a cell",
                "Cell radius against memory, memory-limit ring against usage, membrane wobble "
                "against pressure and CPU arc against quota use.")
    # (a) radius against memory
    a = S.Axes(svg, 80, 60, 360, 220, (1, 65536), (0, 24), xlog=True)
    a.frame(xticks=[1, 16, 256, 4096, 65536], yticks=[0, 6, 12, 18, 24],
            xlabel="cgroup memory (MiB, log scale)", ylabel="cell radius (world units)",
            xfmt=lambda v: {1: "1 MiB", 16: "16", 256: "256", 4096: "4 GiB", 65536: "64 GiB"}[v])
    svg.text(80, 46, "(a) radius = max(1.3 + 0.16 sqrt(MiB), seats term)", 13, weight="600")
    xs = [2 ** (k / 8.0) for k in range(0, 8 * 16 + 1)]
    a.curve(xs, [1.3 + 0.16 * math.sqrt(x) for x in xs], S.GOLD, 2.2)
    for span, shade in [(4, "#7a8ba8"), (16, "#7a8ba8"), (64, "#7a8ba8")]:
        r = 0.75 * math.sqrt(span + 1) + 1
        a.curve([1, 65536], [r, r], shade, 1.2, dash="4 3")
        svg.text(a.px(65536) - 4, a.py(r) - 4, "%d seats" % span, 10, "end", S.MUTED)
    svg.text(a.px(12), a.py(20), "area follows memory:", 11, fill=S.GOLD)
    svg.text(a.px(12), a.py(20) + 14, "r grows as sqrt(memory)", 11, fill=S.GOLD)

    # (b) limit ring
    b = S.Axes(svg, 560, 60, 360, 220, (0, 1.0), (0.9, 1.7))
    b.frame(xticks=[0, 0.25, 0.5, 0.75, 1.0], yticks=[1.0, 1.2, 1.4, 1.6], xlabel="memory / memory.max",
            ylabel="ring radius / cell radius")
    svg.text(560, 46, "(b) limit ring: r sqrt(1 / used), clamped 1.04 to 1.6", 13, weight="600")
    xs = [0.25 + 0.75 * k / 150 for k in range(151)]
    ys = [min(max(1 / math.sqrt(x), 1.04), 1.6) for x in xs]
    for index in range(len(xs) - 1):
        used = (xs[index] + xs[index + 1]) / 2
        t = min(max((used - 0.6) / 0.4, 0.0), 1.0)
        colour = "#%02x%02x%02x" % tuple(int(c0 + (c1 - c0) * t) for c0, c1 in
                                          zip((236, 190, 110), (255, 96, 80)))
        svg.line(b.px(xs[index]), b.py(ys[index]), b.px(xs[index + 1]), b.py(ys[index + 1]),
                 colour, 2.6)
    svg.line(b.px(0.25), b.py(0.9), b.px(0.25), b.py(1.7), S.MUTED, 1, dash="3 3")
    svg.text(b.px(0.25) + 6, b.py(1.12), "no ring below 25 %", 10.5, fill=S.MUTED)
    svg.text(b.px(0.62), b.py(1.5), "colour: amber to red", 11, fill=S.GOLD)
    svg.text(b.px(0.62), b.py(1.5) + 14, "from 60 % to 100 %", 11, fill=S.GOLD)

    # (c) wobble
    c = S.Axes(svg, 80, 400, 360, 220, (0, 100), (0, 0.3))
    c.frame(xticks=[0, 15, 25, 50, 75, 100], yticks=[0, 0.1, 0.2, 0.3],
            xlabel="pressure p (% of time stalled, avg10)", ylabel="wobble amplitude / radius")
    svg.text(80, 386, "(c) wobble = 0.03 + 0.22 p / (p + 15)", 13, weight="600")
    xs = [k for k in range(0, 101)]
    c.curve(xs, [0.03 + 0.22 * x / (x + 15) for x in xs], S.TEAL, 2.2)
    svg.circle(c.px(15), c.py(0.03 + 0.11), 3.5, S.TEAL, "#0e1116", 1)
    svg.text(c.px(15) + 8, c.py(0.14) + 14, "p = 15: half of the swing (0.14)", 11, fill=S.TEAL)
    svg.text(c.px(100) - 4, c.py(0.03) - 6, "floor 0.03 at p = 0", 11, "end", S.MUTED)

    # (d) CPU arc
    d = S.Axes(svg, 560, 400, 360, 220, (0, 2.0), (0, 1.1))
    d.frame(xticks=[0, 0.5, 0.9, 1.0, 1.5, 2.0], yticks=[0, 0.5, 1.0],
            xlabel="cgroup CPU use / quota", ylabel="arc sweep (turns)")
    svg.text(560, 386, "(d) CPU arc: min(cpu / quota, 1) of a turn", 13, weight="600")
    xs = [2.0 * k / 200 for k in range(201)]
    ys = [min(x, 1.0) for x in xs]
    d.curve(xs, ys, S.GOLD, 2.2)
    d.curve([0.9, 2.0], [0.9, 1.0], S.RED, 0.0)
    svg.line(d.px(0.9), d.py(0), d.px(0.9), d.py(1.1), S.RED, 1, dash="3 3")
    svg.text(d.px(0.9) + 6, d.py(0.3), "red above 0.9", 11, fill=S.RED)
    svg.text(d.px(0.9) + 6, d.py(0.3) + 14, "(only with a quota)", 11, fill=S.RED)
    svg.text(d.px(2.0) - 4, d.py(1.0) - 8, "no quota: share of one core", 11, "end", S.MUTED)
    print(svg.save("cells-transfer"))


if __name__ == "__main__":
    main()
