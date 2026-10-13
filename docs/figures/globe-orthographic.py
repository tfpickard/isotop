"""Orthographic projection of the globe, as isotop's camera draws it.

Writes docs/media/figures/globe-orthographic.svg. Run from the repository root:

    python3 docs/figures/globe-orthographic.py

Left: the plane through the view axis and the centre of the earth. Rays are parallel, so the
image of the sphere is a disc of the sphere's own radius whatever the distance. A surface point
with unit normal n is on the near side when n . v >= 0 (v points to the viewer). A point above
the surface at offset d from the centre is hidden by the earth when d . v < 0 and its distance
from the view axis, sqrt(d.d - (d.v)^2), is at most R: the `facing` test of src/globe.rs.

Right: the graticule of the 30 degree grid seen from latitude 35.26 degrees, the default pitch
(asin(1 / sqrt 3)), with the standard orthographic equations for a view centred on (phi0, lambda0):

    x = cos(phi) sin(lambda - lambda0)
    y = cos(phi0) sin(phi) - sin(phi0) cos(phi) cos(lambda - lambda0)
    cos(c) = sin(phi0) sin(phi) + cos(phi0) cos(phi) cos(lambda - lambda0) >= 0  (visible)

which is what `Camera::view` computes for a point at azimuth `spin - longitude` when the pitch is
phi0 (see the page for the algebra).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

PHI0 = math.asin(1 / math.sqrt(3))


def graticule_point(phi, dlam, phi0=PHI0):
    x = math.cos(phi) * math.sin(dlam)
    y = math.cos(phi0) * math.sin(phi) - math.sin(phi0) * math.cos(phi) * math.cos(dlam)
    c = math.sin(phi0) * math.sin(phi) + math.cos(phi0) * math.cos(phi) * math.cos(dlam)
    return x, y, c


def main():
    svg = S.Svg(980, 500, "Orthographic projection of the globe",
                "Parallel rays, the visible hemisphere and the hidden test on the left; the "
                "foreshortened graticule on the right.")
    # Left panel: side view.
    cx, cy, R = 200.0, 262.0, 130.0
    svg.text(30, 34, "Side view: the plane through the view axis", 14, weight="600")
    svg.circle(cx, cy, R, "#12253b", S.AXIS, 1.2)
    # Far half dashed, near half bright.
    svg.path("M %s %s A %s %s 0 0 1 %s %s" % (S.num(cx), S.num(cy - R), R, R, S.num(cx), S.num(cy + R)),
             S.CYAN, 2.2)
    svg.path("M %s %s A %s %s 0 0 0 %s %s" % (S.num(cx), S.num(cy - R), R, R, S.num(cx), S.num(cy + R)),
             S.MUTED, 1.2, dash="4 4")
    svg.line(cx - R - 20, cy, cx + R + 60, cy, S.AXIS, 1, dash="2 3")
    svg.text(cx + R + 62, cy - 6, "view axis", 11, "end", S.MUTED)
    # Projection plane, to the right of the sphere.
    plane = cx + R + 70
    svg.line(plane, cy - R - 40, plane, cy + R + 40, S.GOLD, 1.5)
    svg.text(plane + 8, cy - R - 28, "screen", 11, fill=S.GOLD)
    # Parallel rays towards the viewer.
    for k in (-1.1, -0.7, -0.3, 0.3, 0.7, 1.1):
        y = cy + R * k
        svg.line(cx - R * 0.2, y, plane, y, S.TEXT, 0.8, dash="2 4", opacity=0.5)
    svg.arrow(plane - 90, cy + R + 22, plane - 10, cy + R + 22, S.TEXT, 1.2)
    svg.text(plane - 50, cy + R + 38, "to the viewer, v", 11, "middle", S.TEXT)

    def at(angle_deg, radius=R):
        a = math.radians(angle_deg)
        return (cx + radius * math.cos(a), cy - radius * math.sin(a))

    # A visible surface point, a hidden one, and an arc apex above the far limb.
    examples = [(50, 1.0, "visible: n.v > 0", S.GREEN),
                (-130, 1.0, "behind: hidden", S.RED),
                (200, 1.0 + 0.0, "", S.RED)]
    px, py = at(50)
    svg.circle(px, py, 4.5, S.GREEN, "#0e1116", 1)
    svg.line(px, py, plane, py, S.GREEN, 1, dash="3 3")
    svg.circle(plane, py, 3.5, S.GREEN, "#0e1116", 1)
    svg.text(px + 10, py + 17, "A: n.v > 0, visible", 11, fill=S.GREEN)
    qx, qy = at(-150)
    svg.circle(qx, qy, 4.5, S.RED, "#0e1116", 1)
    svg.line(qx, qy, plane, qy, S.RED, 1, dash="3 3")
    svg.text(qx - 6, qy + 18, "B: behind, inside the disc: hidden", 11, "start", S.RED)
    # An apex above the sphere behind the limb but outside the cylinder: visible.
    ex, ey = cx - R * 0.45, cy - R * 1.3
    svg.circle(ex, ey, 4.5, S.GOLD, "#0e1116", 1)
    svg.line(ex, ey, plane, ey, S.GOLD, 1, dash="3 3")
    svg.text(ex + 10, ey - 6, "C: behind the centre plane, but outside", 11, "start", S.GOLD)
    svg.text(ex + 10, ey + 8, "the silhouette: visible", 11, "start", S.GOLD)
    svg.line(cx, cy - R, plane, cy - R, S.CYAN, 1, dash="1 3", opacity=0.7)
    svg.line(cx, cy + R, plane, cy + R, S.CYAN, 1, dash="1 3", opacity=0.7)
    svg.text(plane - 6, cy - R - 6, "silhouette: radius R", 11, "end", S.CYAN)
    svg.text(cx, cy + R + 66, "disc radius on screen = R * zoom * sqrt(3/2) pixels", 11, "middle", S.MUTED)
    svg.text(cx, cy + R + 84, "(Camera: SCALE = 1.224745, independent of distance)", 11, "middle", S.MUTED)

    # Right panel: graticule.
    gx, gy, G = 740.0, 245.0, 150.0
    svg.text(560, 34, "The graticule seen from latitude 35.26 degrees", 14, weight="600")
    svg.circle(gx, gy, G, "#12253b", S.AXIS, 1.2)
    lat_lines = [-60, -30, 0, 30, 60]
    for lat in lat_lines:
        pts = []
        for step in range(0, 361):
            dlam = math.radians(step - 180)
            x, y, c = graticule_point(math.radians(lat), dlam)
            if c >= 0:
                pts.append((gx + G * x, gy - G * y))
            else:
                if len(pts) > 1:
                    svg.polyline(pts, "#3a6a96", 1.2)
                pts = []
        if len(pts) > 1:
            svg.polyline(pts, "#3a6a96", 1.2)
    for lon in range(-180, 180, 30):
        pts = []
        for step in range(-80, 81):
            x, y, c = graticule_point(math.radians(step), math.radians(lon))
            if c >= 0:
                pts.append((gx + G * x, gy - G * y))
            else:
                if len(pts) > 1:
                    svg.polyline(pts, "#3a6a96", 1.2)
                pts = []
        if len(pts) > 1:
            svg.polyline(pts, "#3a6a96", 1.2)
    # The point under the viewer, and the north pole.
    x0, y0, _ = graticule_point(PHI0, 0.0)
    svg.circle(gx + G * x0, gy - G * y0, 4, S.GOLD, "#0e1116", 1)
    svg.text(gx + G * x0 + 8, gy - G * y0 + 4, "sub-viewer point", 11, fill=S.GOLD)
    xp, yp, _ = graticule_point(math.pi / 2, 0.0)
    svg.circle(gx + G * xp, gy - G * yp, 3.5, S.TEXT, "#0e1116", 1)
    svg.text(gx + G * xp + 8, gy - G * yp - 4, "north pole", 11)
    svg.text(gx, gy + G + 26, "x = cos(phi) sin(dlam),  y = cos(phi0) sin(phi) - sin(phi0) cos(phi) cos(dlam)",
             11, "middle", S.MUTED)
    svg.text(gx, gy + G + 44, "east-west spacing shrinks as cos(phi) sin(dlam) flattens at the limb",
             11, "middle", S.MUTED)
    print(svg.save("globe-orthographic"))


if __name__ == "__main__":
    main()
