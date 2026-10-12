"""Strata's time axis: how much history the ring keeps, and how the ridge scrolls between samples.

Writes docs/media/figures/strata-window.svg. Run from the repository root:

    python3 docs/figures/strata-window.py

  (a) `History` keeps CAPACITY = 240 samples (src/history.rs). Strata asks for the samples whose
      `elapsed` lies in the last WINDOW = 60 s (src/strata.rs). The seconds of history the ring
      can actually answer with are min(60, 240 * sample interval); below 250 ms the minute is
      truncated (issue #127).
  (b) the number of points one ridge has in its minute is min(240, 60 / interval + 1) (the
      window is closed at both ends), which is the horizontal resolution of the ridge.
  (c) `Strata::draw` puts a sample at x = (elapsed - now - scroll) * SPAN. While the newest
      sample is the shown one, `scroll` runs from 0 to the sampling interval since it arrived,
      the last value is held out to x = 0, and the landscape slides smoothly instead of stepping.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

CAPACITY, WINDOW = 240, 60.0


def main():
    svg = S.Svg(980, 700, "Strata's time axis",
                "History coverage and ridge resolution against the sampling interval, and the "
                "smooth scroll between samples.")
    ms = [100 * 10 ** (k / 40.0) for k in range(0, 81)]
    coverage = [min(WINDOW, CAPACITY * m / 1000.0) for m in ms]
    points = [min(CAPACITY, WINDOW / (m / 1000.0) + 1) for m in ms]
    ticks = [100, 250, 1000, 10000]

    a = S.Axes(svg, 80, 60, 360, 220, (100, 10000), (0, 70), xlog=True)
    a.frame(xticks=ticks, yticks=[0, 20, 40, 60], xlabel="--sample-ms (log scale)",
            ylabel="seconds of history in the ring")
    svg.text(80, 46, "(a) the minute Strata can draw", 13, weight="600")
    short = [m for m in ms if m <= 250.0001]
    svg.polygon([a.pt(m, min(WINDOW, CAPACITY * m / 1000.0)) for m in short] + [a.pt(250, 60), a.pt(100, 60)],
                fill=S.RED, opacity=0.16)
    a.curve(ms, coverage, S.GOLD, 2.4)
    a.curve([100, 10000], [60, 60], S.MUTED, 1, dash="4 3")
    svg.text(a.px(10000) - 4, a.py(60) - 6, "WINDOW = 60 s", 11, "end", S.MUTED)
    svg.line(a.px(250), a.py(0), a.px(250), a.py(70), S.RED, 1, dash="3 3")
    svg.text(a.px(250) + 6, a.py(14), "250 ms: 240 samples", 11, fill=S.RED)
    svg.text(a.px(250) + 6, a.py(14) + 14, "= 60 s exactly", 11, fill=S.RED)
    svg.circle(a.px(100), a.py(24), 3.5, S.GOLD, "#0e1116", 1)
    svg.text(a.px(100) + 8, a.py(24) - 6, "24 s at 100 ms", 11, fill=S.GOLD)

    b = S.Axes(svg, 560, 60, 360, 220, (100, 10000), (1, 300), xlog=True, ylog=True)
    b.frame(xticks=ticks, yticks=[1, 10, 100, 240], xlabel="--sample-ms (log scale)",
            ylabel="points per ridge in its minute")
    svg.text(560, 46, "(b) horizontal resolution of a ridge", 13, weight="600")
    b.curve(ms, points, S.TEAL, 2.4)
    svg.text(b.px(1000) + 6, b.py(61) - 6, "61 points at 1 s", 11, fill=S.TEAL)
    svg.circle(b.px(1000), b.py(61), 3.5, S.TEAL, "#0e1116", 1)
    svg.text(b.px(10000) - 4, b.py(7) - 8, "7 points at 10 s", 11, "end", S.TEAL)

    # (c) the scroll between samples
    svg.text(80, 346, "(c) x = (elapsed - now - scroll) * SPAN: the newest segment is a held value", 13,
             weight="600")
    values = [0.2, 0.45, 0.3, 0.7, 0.95, 0.6, 0.35, 0.5, 0.8]
    interval = 1.0
    frames = [("just after a sample (scroll = 0)", 0.0),
              ("half an interval later (scroll = 0.5)", 0.5),
              ("just before the next (scroll = 1)", 1.0)]
    for index, (label, scroll) in enumerate(frames):
        top = 380 + index * 100
        ax = S.Axes(svg, 80, top, 840, 70, (-8.4, 0.6), (0, 1.1))
        svg.rect(ax.left, ax.top, ax.width, ax.height, "none", S.AXIS, 1)
        n = len(values)
        pts = []
        for k, v in enumerate(values):
            x = (k - (n - 1)) * interval - scroll
            pts.append((x, v))
        profile = [p for p in pts if p[0] >= -8.0]
        held = profile + [(0.0, profile[-1][1])]
        svg.polygon([ax.pt(held[0][0], 0)] + [ax.pt(x, y) for x, y in held] + [ax.pt(0, 0)],
                    fill=S.TEAL, opacity=0.25)
        svg.polyline([ax.pt(x, y) for x, y in held], S.TEAL, 2)
        for x, y in profile:
            svg.circle(ax.px(x), ax.py(y), 3.2, S.TEXT, "#0e1116", 0.8)
        svg.line(ax.px(0), ax.top, ax.px(0), ax.top + ax.height, S.GOLD, 1.2, dash="4 3")
        svg.text(ax.px(0) + 4, ax.top + 12, "now", 11, fill=S.GOLD)
        svg.text(ax.left + 6, ax.top + 14, label, 11, fill=S.MUTED)
    print(svg.save("strata-window"))
    print("coverage at 100 ms", min(WINDOW, CAPACITY * 0.1), "points at 1 s", WINDOW / 1.0 + 1)


if __name__ == "__main__":
    main()
