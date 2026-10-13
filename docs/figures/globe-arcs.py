"""Great-circle arcs on the globe, with the real coastline file and the arc rule of src/globe.rs.

Writes docs/media/figures/globe-arcs.svg. Run from the repository root:

    python3 docs/figures/globe-arcs.py

Left: the coastlines of src/coastline.bin (read here with the same layout the renderer uses: per
line a little-endian u16 point count, then i16 pairs of longitude and latitude in hundredths of
a degree), orthographic from latitude 35.26 degrees (the default camera) over Amsterdam's
longitude, and arcs from Amsterdam (the demo's home) to six cities and to one address with no
place, which circles the north pole at 1.25 R. Each arc is, with Omega the angle between the unit
vectors `a` (home) and `b` (the endpoint) and t from 0 to 1,

    d(t) = sin((1 - t) Omega) / sin Omega * a + sin(t Omega) / sin Omega * b        (slerp)
    r(t) = R * (1.01 + (h - 1.01) t) + R * (0.04 + 0.22 Omega / pi) * sin(pi t)

with h = 1.01 for a placed endpoint and 1.25 for an unplaced one. Right: that radial profile for
three Omegas, and why slerp rather than normalised linear interpolation: the latter does not
sweep the angle at a constant rate.
"""

import math
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

PHI0 = math.asin(1 / math.sqrt(3))
HOME = (52.37, 4.9, "Amsterdam")
CITIES = [(37.77, -122.42, "San Francisco"), (35.68, 139.69, "Tokyo"),
          (-33.87, 151.21, "Sydney"), (-23.55, -46.63, "Sao Paulo"),
          (-26.20, 28.05, "Johannesburg"), (1.35, 103.82, "Singapore")]


def coastline():
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "src",
                        "coastline.bin")
    with open(path, "rb") as handle:
        data = handle.read()
    lines, at = [], 0
    while at + 2 <= len(data):
        (count,) = struct.unpack_from("<H", data, at)
        at += 2
        if at + count * 4 > len(data):
            break
        lines.append([(struct.unpack_from("<h", data, at + 4 * k)[0] / 100.0,
                       struct.unpack_from("<h", data, at + 4 * k + 2)[0] / 100.0)
                      for k in range(count)])
        at += count * 4
    return lines


def unit(lat, lon):
    phi, lam = math.radians(lat), math.radians(lon)
    return (math.cos(phi) * math.cos(lam), math.cos(phi) * math.sin(lam), math.sin(phi))


def view(n, rho, lam0):
    """Orthographic coordinates of the point rho * n for a view centred on (PHI0, lam0)."""
    phi = math.asin(max(-1.0, min(1.0, n[2])))
    lam = math.atan2(n[1], n[0])
    d = lam - lam0
    x = rho * math.cos(phi) * math.sin(d)
    y = rho * (math.cos(PHI0) * math.sin(phi) - math.sin(PHI0) * math.cos(phi) * math.cos(d))
    w = rho * (math.sin(PHI0) * math.sin(phi) + math.cos(PHI0) * math.cos(phi) * math.cos(d))
    return x, y, w


def facing(x, y, w, rho_unit=1.0):
    return w > 0 or math.hypot(x, y) > rho_unit


def slerp(a, b, omega, t):
    if omega < 1e-4:
        return a
    wa = math.sin((1 - t) * omega) / math.sin(omega)
    wb = math.sin(t * omega) / math.sin(omega)
    v = [a[k] * wa + b[k] * wb for k in range(3)]
    length = math.sqrt(sum(c * c for c in v))
    return tuple(c / length for c in v)


def main():
    lam0 = math.radians(HOME[1])
    svg = S.Svg(1060, 560, "Great-circle arcs on the globe",
                "Coastlines from the packed file, arcs from Amsterdam by spherical interpolation, "
                "and the radial profile of an arc.")
    gx, gy, G = 290.0, 300.0, 230.0
    svg.text(30, 34, "Home = Amsterdam (the demo's), viewed from 35.26 degrees north", 14, weight="600")
    svg.circle(gx, gy, G, "#12253b", S.AXIS, 1.2)
    # Graticule.
    for lon in range(-180, 180, 30):
        pts = []
        for lat in range(-80, 81, 4):
            x, y, w = view(unit(lat, lon), 1.002, lam0)
            if w >= 0:
                pts.append((gx + G * x, gy - G * y))
            elif len(pts) > 1:
                svg.polyline(pts, "#274c70", 0.8)
                pts = []
            else:
                pts = []
        if len(pts) > 1:
            svg.polyline(pts, "#274c70", 0.8)
    for lat in (-60, -30, 0, 30, 60):
        pts = []
        for lon in range(-180, 181, 4):
            x, y, w = view(unit(lat, lon), 1.002, lam0)
            if w >= 0:
                pts.append((gx + G * x, gy - G * y))
            elif len(pts) > 1:
                svg.polyline(pts, "#274c70", 0.8)
                pts = []
            else:
                pts = []
        if len(pts) > 1:
            svg.polyline(pts, "#274c70", 0.8)
    # Coastlines.
    for line in coastline():
        pts = []
        for lon, lat in line:
            x, y, w = view(unit(lat, lon), 1.004, lam0)
            if w >= 0:
                pts.append((gx + G * x, gy - G * y))
            else:
                if len(pts) > 1:
                    svg.polyline(pts, "#8cc4a6", 0.9)
                pts = []
        if len(pts) > 1:
            svg.polyline(pts, "#8cc4a6", 0.9)
    # Arcs.
    home = unit(HOME[0], HOME[1])
    targets = [(unit(lat, lon), 1.01, name) for lat, lon, name in CITIES]
    angle = math.radians(40)
    pole_ring = (0.45 * math.cos(angle), 0.45 * math.sin(angle), 1.0)
    length = math.sqrt(sum(c * c for c in pole_ring))
    targets.append((tuple(c / length for c in pole_ring), 1.25, "unplaced address"))
    for target, height, name in targets:
        omega = math.acos(max(-1.0, min(1.0, sum(home[k] * target[k] for k in range(3)))))
        pts, run = [], []
        for k in range(0, 41):
            t = k / 40.0
            d = slerp(home, target, omega, t)
            rho = (1.01 + (height - 1.01) * t) + (0.04 + 0.22 * omega / math.pi) * math.sin(math.pi * t)
            x, y, w = view(d, rho, lam0)
            if facing(x, y, w):
                run.append((gx + G * x, gy - G * y))
            else:
                if len(run) > 1:
                    pts.append(run)
                run = []
        if len(run) > 1:
            pts.append(run)
        for run in pts:
            svg.polyline(run, S.CYAN, 1.5, opacity=0.9)
        x, y, w = view(target, height, lam0)
        if facing(x, y, w):
            svg.circle(gx + G * x, gy - G * y, 3.4, S.CYAN, "#0e1116", 0.8)
            svg.text(gx + G * x + 6, gy - G * y - 5, name, 10.5, fill=S.TEXT)
    hx, hy, _ = view(home, 1.01, lam0)
    svg.circle(gx + G * hx, gy - G * hy, 4.5, "#ffd696", "#0e1116", 1)
    svg.text(gx + G * hx - 8, gy - G * hy + 16, "home", 11, "end", "#ffd696")

    # Right panels.
    a = S.Axes(svg, 700, 70, 320, 200, (0, 1), (1.0, 1.3))
    a.frame(xticks=[0, 0.25, 0.5, 0.75, 1], yticks=[1.0, 1.1, 1.2, 1.3], xlabel="t along the arc",
            ylabel="radius / R")
    svg.text(700, 56, "Radial profile of a placed endpoint", 13, weight="600")
    for deg, colour in ((30, S.TEAL), (90, S.GOLD), (180, S.PINK)):
        omega = math.radians(deg)
        xs = [k / 100.0 for k in range(101)]
        a.curve(xs, [1.01 + (0.04 + 0.22 * omega / math.pi) * math.sin(math.pi * x) for x in xs],
                colour, 2.0)
        peak = 1.01 + 0.04 + 0.22 * omega / math.pi
        svg.text(a.px(0.5), a.py(peak) - 6, "Omega = %d deg: peak %.3f R" % (deg, peak), 10.5,
                 "middle", colour)

    b = S.Axes(svg, 700, 340, 320, 170, (0, 1), (0, 150))
    b.frame(xticks=[0, 0.25, 0.5, 0.75, 1], yticks=[0, 50, 100, 150], xlabel="t",
            ylabel="angle from home (deg)")
    svg.text(700, 326, "Why slerp: Omega = 150 deg", 13, weight="600")
    omega = math.radians(150)
    xs = [k / 100.0 for k in range(101)]
    b.curve(xs, [150 * x for x in xs], S.GOLD, 2.0)
    b.curve(xs, [math.degrees(math.atan2(x * math.sin(omega), (1 - x) + x * math.cos(omega)))
                 for x in xs], S.RED, 2.0, dash="5 3")
    svg.text(b.px(0.04), b.py(130), "slerp: uniform", 11, fill=S.GOLD)
    svg.text(b.px(0.55), b.py(40), "normalised lerp:", 11, fill=S.RED)
    svg.text(b.px(0.55), b.py(40) + 14, "speeds up in the middle", 11, fill=S.RED)
    print(svg.save("globe-arcs"))


if __name__ == "__main__":
    main()
