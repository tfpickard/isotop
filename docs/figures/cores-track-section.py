"""The Cores track in plan and in section, from the constants of src/cores.rs.

Writes docs/media/figures/cores-track-section.svg. Run from the repository root:

    python3 docs/figures/cores-track-section.py

Lane radii follow `Track::draw`: the first lane sits at INNER, each lane is LANE wide, and a gap of
GROUP_GAP opens where the core kind changes. Height follows the banking `z = BANK * (r - INNER)`.
The demo machine has eight performance and eight efficiency CPUs.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

INNER, LANE, GROUP_GAP, BANK = 9.0, 1.5, 1.2, 0.16
KINDS = ["P"] * 8 + ["E"] * 8


def lanes():
    radius, previous, found = INNER, None, []
    for index, kind in enumerate(KINDS):
        if previous is not None and kind != previous:
            radius += GROUP_GAP
        found.append((index, kind, radius))
        radius += LANE
        previous = kind
    return found, radius - LANE * 0.5


def main():
    found, outer = lanes()
    svg = S.Svg(980, 560, "The Cores track in plan and section",
                "Concentric lanes, performance cores inside and efficiency cores outside, banked "
                "so that outer lanes stand higher.")
    colour = {"P": S.GOLD, "E": S.TEAL}

    # Plan view, seen from above with x to the right and y up.
    cx, cy, scale = 245, 270, 6.2

    def at(angle, radius):
        return (cx + radius * scale * math.cos(angle), cy - radius * scale * math.sin(angle))

    svg.text(30, 34, "Plan: one lane per CPU, seen from above", 14, weight="600")
    svg.circle(cx, cy, (INNER - LANE * 0.5 - 0.4) * scale, "#0c121c", S.AXIS, 1)
    for index, kind, radius in found:
        svg.circle(cx, cy, radius * scale, "none", colour[kind], LANE * scale * 0.82, opacity=0.55)
    svg.circle(cx, cy, outer * scale, "none", S.AXIS, 1.2)
    # The start line at angle 0.
    a, b = at(0, INNER - LANE * 0.5 - 0.4), at(0, outer)
    svg.line(a[0], a[1], b[0], b[1], "#e6ecf0", 1.6)
    # A marble part-way round lane 3, with the arc it has covered.
    lane = found[3]
    angle = 2.2
    arc = [at(angle - 0.8 + 0.8 * k / 12, lane[2]) for k in range(13)]
    svg.polyline(arc, S.VIOLET, 2.4, opacity=0.8)
    mx, my = at(angle, lane[2])
    svg.circle(mx, my, 6, S.VIOLET, "#0e1116", 1)
    svg.text(mx + 4, my - 12, "marble", 11, "middle", S.VIOLET)
    # Red queue beads sit just behind the start line of a lane with waiting tasks.
    queue = found[10]
    for j in range(3):
        x, y = at(-(0.9 + j * 0.55) / queue[2], queue[2])
        svg.circle(x, y, 3.6, S.RED, "#0e1116", 0.8)
    svg.text(cx, cy + 4, "infield", 11, "middle", S.MUTED)
    svg.text(cx, cy + outer * scale + 26,
             "P lanes r = 9 to 19.5, E lanes r = 22.2 to 32.7, wall at r = %.2f" % outer,
             11, "middle", S.MUTED)
    svg.text(cx, cy + outer * scale + 44,
             "white line = start line; red beads = queue behind it; angle grows counter-clockwise",
             11, "middle", S.MUTED)

    # Section.
    ax = S.Axes(svg, 650, 90, 300, 250, (INNER - 1, outer + 1), (0, 4.5))
    ax.frame(xticks=[10, 15, 20, 25, 30], yticks=[0, 1, 2, 3, 4], xlabel="radius r (world units)",
             ylabel="height z (world units)")
    svg.text(650, 78, "Section: banking z = 0.16 (r - 9)", 14, weight="600")
    xs = [INNER - 1 + k * (outer + 1 - INNER + 1) / 100 for k in range(101)]
    ax.curve(xs, [BANK * max(x - INNER, 0.0) for x in xs], "#e6ecf0", 1.2, opacity=0.7)
    for index, kind, radius in found:
        left, right = radius - LANE * 0.5 + 0.08, radius + LANE * 0.5 - 0.08
        z0, z1 = BANK * (left - INNER), BANK * (right - INNER)
        svg.polygon([ax.pt(left, 0), ax.pt(left, max(z0, 0)), ax.pt(right, max(z1, 0)),
                     ax.pt(right, 0)], fill=colour[kind], opacity=0.55)
    svg.text(ax.px(12.4), ax.py(1.95), "performance band", 11, "middle", S.GOLD)
    svg.text(ax.px(12.4), ax.py(1.95) + 14, "mean r = 14.25", 11, "middle", S.GOLD)
    svg.text(ax.px(24.0), ax.py(3.55), "efficiency band", 11, "middle", S.TEAL)
    svg.text(ax.px(24.0), ax.py(3.55) + 14, "mean r = 27.45", 11, "middle", S.TEAL)
    gap_lo, gap_hi = 19.5 + LANE * 0.5, 22.2 - LANE * 0.5
    svg.text(ax.px((gap_lo + gap_hi) / 2), ax.py(0) - 6, "gap", 10, "middle", S.MUTED)

    svg.text(650, 400, "A marble rides its process's last CPU's lane (Linux)", 12)
    svg.text(650, 418, "or the middle of its cluster's lanes (macOS).", 12)
    svg.text(650, 450, "Equal angular speed is a larger linear speed on an", 12, fill=S.MUTED)
    svg.text(650, 468, "outer lane: lap time, not distance, is the invariant.", 12, fill=S.MUTED)
    print(svg.save("cores-track-section"))


if __name__ == "__main__":
    main()
