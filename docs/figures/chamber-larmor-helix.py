"""Track curvature in a magnetic field: p = 0.3 B r, the sign of the charge, and the helix.

Writes docs/media/figures/chamber-larmor-helix.svg. Run from the repository root:

    python3 docs/figures/chamber-larmor-helix.py

Left: the radius of curvature of a singly charged particle, r = p_T / (0.3 B) with p_T in
GeV/c, B in tesla and r in metres, for three field strengths. Middle: B points out of the
page; a positive charge turns clockwise and a negative one anticlockwise, on circles whose
radius is proportional to the transverse momentum. Right: with a longitudinal momentum the
circle becomes a helix of pitch 2 pi r tan(lambda), drawn in oblique projection.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402


def radius_metres(momentum_gev, field_tesla):
    return momentum_gev / (0.3 * field_tesla)


def main():
    svg = P.Svg(1200, 470, "Larmor radius, charge sign and helix",
                "Radius against momentum for three fields; opposite curl of positive and "
                "negative tracks; a helix in oblique projection.")
    svg.text(24, 28, "A charged track in a uniform field: r = p / (0.3 B)", size=16,
             weight="600")
    svg.text(24, 47, "p in GeV/c, B in tesla, r in metres. Derivation: q v B = m gamma v^2 / r "
             "gives p = q B r; with q = e, p[GeV/c] = 0.29979 B[T] r[m].", size=11,
             fill=P.MUTED)

    top, w, h = 100, 340, 290
    momenta = [10 ** (k / 20.0) for k in range(-40, 41)]
    axes = P.Axes(svg, 70, top, w, h, (0.01, 100), (0.001, 1000), xlog=True, ylog=True,
                  xlabel="transverse momentum p (GeV/c)", ylabel="radius r (m)",
                  title="Radius of curvature, q = e",
                  xticks=[0.01, 0.1, 1, 10, 100], yticks=[0.001, 0.01, 0.1, 1, 10, 100, 1000])
    for field, colour in ((0.5, P.MUTED), (1.5, P.BLUE), (3.5, P.ORANGE)):
        axes.plot(momenta, [radius_metres(p, field) for p in momenta], stroke=colour, width=2.0)
    svg.legend(95, top + h - 45, [("B = 0.5 T", P.MUTED, "line"), ("1.5 T", P.BLUE, "line"),
                                  ("3.5 T", P.ORANGE, "line")], size=10.5, gap=15)
    axes.dots([1.0], [radius_metres(1.0, 1.5)], fill=P.BLUE, radius=3.5)
    axes.label(1.0, radius_metres(1.0, 1.5), "1 GeV/c in 1.5 T: 2.22 m", size=10.5, dx=-6,
               dy=-8, anchor="end")

    # Transverse plane: guiding centres and circles. B out of the page.
    cx0, cy0 = 520, top
    svg.rect(cx0, cy0, 330, h, stroke=P.AXIS, width=1.0)
    svg.text(cx0, cy0 - 9, "Transverse plane, B out of the page", size=13, weight="600")
    scale = 52.0
    tracks = [(0.9, +1, (80, 200)), (0.5, -1, (225, 215)), (0.3, +1, (265, 75)),
              (0.7, -1, (85, 70))]
    for momentum, charge, (gx, gy) in tracks:
        radius = momentum * scale
        centre = (cx0 + gx, cy0 + gy)
        colour = P.BLUE if charge > 0 else P.ORANGE
        svg.circle(centre[0], centre[1], radius, stroke=colour, width=2.0, stroke_opacity=0.9)
        svg.circle(centre[0], centre[1], 2.2, fill=P.MUTED)
        # Arrow head on the circle: clockwise for positive charge, anticlockwise for negative.
        angle = -0.6
        px, py = centre[0] + radius * math.cos(angle), centre[1] + radius * math.sin(angle)
        # tangent direction in screen coordinates (y down); clockwise on screen = +angle
        sign = 1.0 if charge > 0 else -1.0
        tx, ty = -math.sin(angle) * sign, math.cos(angle) * sign
        nx, ny = -ty, tx
        head = [(px + tx * 7, py + ty * 7), (px - tx * 3 + nx * 5, py - ty * 3 + ny * 5),
                (px - tx * 3 - nx * 5, py - ty * 3 - ny * 5)]
        svg.polyline(head, stroke=colour, fill=colour, close=True, width=1.0)
        svg.text(centre[0], centre[1] + radius + 15, "%+d, p = %.1f" % (charge, momentum),
                 size=10.5, anchor="middle", fill=colour)
    svg.text(cx0 + 12, cy0 + h - 12, "radius is proportional to p; sense is set by the sign of q",
             size=10.5, fill=P.MUTED)

    # Helix in oblique projection.
    hx0 = 890
    svg.rect(hx0, top, 290, h, stroke=P.AXIS, width=1.0)
    svg.text(hx0, top - 9, "With p_z: a helix (oblique view)", size=13, weight="600")
    radius, pitch, turns = 42.0, 42.0, 3.4
    points = []
    steps = 360
    for k in range(steps + 1):
        phi = turns * 2 * math.pi * k / steps
        x = radius * math.cos(-phi)
        y = radius * math.sin(-phi)
        z = pitch * phi / (2 * math.pi)
        sx = hx0 + 70 + x + 0.55 * z * 0.5
        sy = top + h - 60 - (0.55 * z * 0.87) + y * 0.45
        points.append((sx, sy))
    svg.polyline(points, stroke=P.AQUA, width=2.0)
    # Projection of the helix on the transverse plane (the circle) and the axis.
    ground = []
    for k in range(121):
        phi = 2 * math.pi * k / 120
        ground.append((hx0 + 70 + radius * math.cos(phi), top + h - 60 + radius * math.sin(phi) * 0.45))
    svg.polyline(ground, stroke=P.MUTED, width=1.0, dash="3 3")
    svg.line(hx0 + 70, top + h - 60, hx0 + 70 + 0.55 * pitch * turns * 0.5,
             top + h - 60 - 0.55 * pitch * turns * 0.87, stroke=P.MUTED, width=1.0, dash="3 3")
    svg.text(hx0 + 14, top + h - 14, "pitch = 2 pi r tan(lambda), tan(lambda) = p_z / p_T",
             size=10.5, fill=P.MUTED)
    svg.text(hx0 + 150, top + 28, "p_z > 0: rises", size=10.5, fill=P.AQUA)
    svg.save("docs/media/figures/chamber-larmor-helix.svg")


if __name__ == "__main__":
    main()
