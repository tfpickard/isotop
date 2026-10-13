"""The macOS cluster marble as a Schmitt trigger.

Writes docs/media/figures/cores-hysteresis.svg. Run from the repository root:

    python3 docs/figures/cores-hysteresis.py

On macOS a process has no last CPU, only `performance_share`: the fraction of its CPU time in the
last interval that ran on performance cores. src/cores.rs moves its marble to the performance
band when the share reaches TO_PERFORMANCE (0.6), to the efficiency band when it falls to
TO_EFFICIENCY (0.4) or below, and leaves it where it is in between and whenever the process used
no CPU time (share None). A new marble starts on the band of the larger part (FIRST_LEVEL 0.5).
The share series below is made up (a slow swing plus a fast ripple, with three idle samples); the
rule is the code's, the naive rule compares with 0.5 alone.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

TO_PERFORMANCE, TO_EFFICIENCY, FIRST_LEVEL = 0.6, 0.4, 0.5
COUNT = 40
IDLE = {13, 14, 29}


def series():
    values = []
    for k in range(COUNT):
        if k in IDLE:
            values.append(None)
            continue
        value = 0.5 + 0.34 * math.sin(0.33 * k) + 0.13 * math.sin(1.9 * k + 1.0)
        values.append(min(max(value, 0.0), 1.0))
    return values


def run(values):
    """Band per sample (1 performance, 0 efficiency) and hop count, as `Track::draw` decides."""
    level, hops, bands = None, 0, []
    for share in values:
        if level is None:
            level = 1 if (share if share is not None else 0.0) >= FIRST_LEVEL else 0
        elif share is not None:
            if share >= TO_PERFORMANCE:
                new = 1
            elif share <= TO_EFFICIENCY:
                new = 0
            else:
                new = level
            if new != level:
                hops += 1
            level = new
        bands.append(level)
    return bands, hops


def naive(values):
    level, flips, bands = None, 0, []
    for share in values:
        if share is not None:
            new = 1 if share >= 0.5 else 0
            if level is not None and new != level:
                flips += 1
            level = new
        bands.append(0 if level is None else level)
    return bands, flips


def main():
    values = series()
    bands, hops = run(values)
    plain, flips = naive(values)
    svg = S.Svg(900, 560, "Hysteresis of a cluster marble",
                "Performance share per sample, the two thresholds, and the band the marble rides "
                "with and without hysteresis.")
    left, width = 80, 780
    ax = S.Axes(svg, left, 60, width, 200, (-0.5, COUNT - 0.5), (0, 1))
    ax.frame(xticks=list(range(0, COUNT, 5)), yticks=[0, 0.4, 0.5, 0.6, 1.0], xlabel="",
             ylabel="performance share")
    svg.text(left, 46, "Share of CPU time on performance cores, one value per sample", 13,
             weight="600")
    svg.polygon([ax.pt(-0.5, 0.6), ax.pt(COUNT - 0.5, 0.6), ax.pt(COUNT - 0.5, 1),
                 ax.pt(-0.5, 1)], fill=S.GOLD, opacity=0.10)
    svg.polygon([ax.pt(-0.5, 0), ax.pt(COUNT - 0.5, 0), ax.pt(COUNT - 0.5, 0.4),
                 ax.pt(-0.5, 0.4)], fill=S.TEAL, opacity=0.10)
    svg.line(ax.px(-0.5), ax.py(0.6), ax.px(COUNT - 0.5), ax.py(0.6), S.GOLD, 1, dash="5 4")
    svg.line(ax.px(-0.5), ax.py(0.4), ax.px(COUNT - 0.5), ax.py(0.4), S.TEAL, 1, dash="5 4")
    svg.text(ax.px(COUNT - 0.5) - 4, ax.py(0.6) - 5, "hop up at 0.6", 11, "end", S.GOLD)
    svg.text(ax.px(COUNT - 0.5) - 4, ax.py(0.4) + 14, "hop down at 0.4", 11, "end", S.TEAL)
    points = [(k, v) for k, v in enumerate(values) if v is not None]
    ax.curve([k for k, _ in points], [v for _, v in points], S.MUTED, 1, opacity=0.6)
    for k, v in enumerate(values):
        if v is None:
            svg.text(ax.px(k), ax.py(0.5) + 4, "x", 12, "middle", S.RED)
        else:
            svg.circle(ax.px(k), ax.py(v), 3.2, S.TEXT, "#0e1116", 0.8)
    svg.text(left + 4, 300, "x = process used no CPU time in the interval: no share, the marble stays",
             11, fill=S.RED)

    low = S.Axes(svg, left, 330, width, 70, (-0.5, COUNT - 0.5), (-0.2, 1.2))
    low.frame(xticks=[], yticks=[0, 1], xlabel="", yfmt=lambda v: "E" if v == 0 else "P",
              grid=False, title=None)
    svg.text(left, 318, "Band ridden with the code's rule: %d hops" % hops, 13, weight="600")
    low.steps(list(range(COUNT)), bands, S.GOLD, 2.2)
    for k in range(COUNT - 1):
        if bands[k] != bands[k + 1]:
            svg.circle(low.px(k + 1), low.py(bands[k + 1]), 3.5, S.GOLD, "#0e1116", 1)

    low2 = S.Axes(svg, left, 450, width, 70, (-0.5, COUNT - 0.5), (-0.2, 1.2))
    low2.frame(xticks=list(range(0, COUNT, 5)), yticks=[0, 1], xlabel="sample",
               yfmt=lambda v: "E" if v == 0 else "P", grid=False)
    svg.text(left, 438, "Band with a single threshold at 0.5: %d flips" % flips, 13, weight="600")
    low2.steps(list(range(COUNT)), plain, S.RED, 2.2)
    print(svg.save("cores-hysteresis"))
    print("hops", hops, "naive flips", flips)


if __name__ == "__main__":
    main()
