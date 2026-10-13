"""Two spirals: a delta ray losing energy in liquid hydrogen, and a process losing CPU.

Writes docs/media/figures/chamber-spirals.svg. Run from the repository root:

    python3 docs/figures/chamber-spirals.py

Left (physics): an electron of 2 MeV kinetic energy in liquid hydrogen at B = 1.5 T. It loses
energy continuously (the electron form of the Bethe formula, collision loss only, no density
correction), its momentum falls, and r = p / (0.3 B) shrinks, so the track spirals in until the
electron stops. Right (the view's specified integration, issue #42): heading turns by
-s v / r H per step with H = 1/120 s, r = K_R cpu clamped to [0.4, 18] world units, speed
v = 6 beta, beta = p / sqrt(p^2 + m^2), m = 1.2 per GiB, and the guiding centre relaxes to the
home position at rate 0.6 / s. The process holds 1 GiB and its CPU decays from 1.6 cores.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

ELECTRON = 0.510999
K = 0.307075
ZA = 0.99212
IONISATION = 21.8e-6
DENSITY = 0.0708


def electron_loss(kinetic):
    """Collision stopping power of an electron in liquid hydrogen, MeV/cm (PDG form)."""
    gamma = 1.0 + kinetic / ELECTRON
    beta2 = 1.0 - 1.0 / gamma**2
    bracket = (math.log(ELECTRON**2 * beta2 * gamma**2 * (gamma - 1.0) / (2.0 * IONISATION**2))
               + (1.0 - beta2) - (2.0 * gamma - 1.0) / gamma**2 * math.log(2.0)
               + (gamma - 1.0) ** 2 / (8.0 * gamma**2))
    return 0.5 * K * ZA / beta2 * bracket * DENSITY


def delta_ray(kinetic0=2.0, field=1.5, floor=0.02):
    x, y, theta = 0.0, 0.0, 0.0
    kinetic = kinetic0
    points = [(x, y)]
    path = 0.0
    while kinetic > floor:
        momentum = math.sqrt(kinetic**2 + 2.0 * kinetic * ELECTRON)
        radius_cm = momentum / (3.0 * field)
        ds = min(0.004, radius_cm * 0.05)
        theta += ds / radius_cm
        x += ds * math.cos(theta)
        y += ds * math.sin(theta)
        kinetic -= electron_loss(kinetic) * ds
        path += ds
        points.append((x, y))
    return points, path


def process_track(seconds=60.0):
    step = 1.0 / 120.0
    c_visual, k_radius, mass_per_gib, k_home = 6.0, 10.5, 1.2, 0.6
    home = (0.0, 0.0)
    sign = 1.0
    cpu0 = 1.6

    def cpu(t):
        return 0.05 + cpu0 * math.exp(-t / 14.0)

    p = cpu(0)
    r = min(max(k_radius * p, 0.4), 18.0)
    x, y, theta = -r, 0.0, math.pi / 2  # on the circle about home, heading up, turning right
    points = []
    for n in range(int(seconds / step)):
        t = n * step
        p = cpu(t)
        if p < 0.10:
            break
        r = min(max(k_radius * p, 0.4), 18.0)
        beta = p / math.sqrt(p * p + mass_per_gib**2)
        v = c_visual * beta
        theta += -sign * v / r * step
        x += v * math.cos(theta) * step
        y += v * math.sin(theta) * step
        cx = x + sign * r * math.sin(theta)
        cy = y - sign * r * math.cos(theta)
        x += (home[0] - cx) * k_home * step
        y += (home[1] - cy) * k_home * step
        if n % 6 == 0:
            points.append((x, y))
    return points, t


def fit(points, left, top, size, margin=12):
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    span = max(max(xs) - min(xs), max(ys) - min(ys)) * 1.08
    scale = (size - 2 * margin) / span
    cx = (max(xs) + min(xs)) / 2
    cy = (max(ys) + min(ys)) / 2
    return [(left + size / 2 + (x - cx) * scale, top + size / 2 - (y - cy) * scale)
            for x, y in points], scale, (cx, cy)


def main():
    svg = P.Svg(1000, 520, "Spirals from falling momentum",
                "A delta ray spiralling in liquid hydrogen, and a process track spiralling "
                "toward its home as its CPU falls.")
    svg.text(24, 28, "Falling momentum, shrinking radius, an inward spiral", size=16,
             weight="600")
    svg.text(24, 47, "r = p / (0.3 B) holds at every instant; only p changes along the track.",
             size=11, fill=P.MUTED)
    size, top = 420, 90

    track, path = delta_ray()
    pts, scale, _ = fit(track, 40, top, size)
    svg.rect(40, top, size, size, stroke=P.AXIS, width=1.0)
    svg.text(40, top - 9, "A 2 MeV delta ray (electron, anticlockwise), liquid H2, B = 1.5 T", size=13,
             weight="600")
    svg.polyline(pts, stroke="#e6edf3", width=1.4)
    svg.circle(pts[0][0], pts[0][1], 3.0, fill=P.ORANGE)
    svg.circle(pts[-1][0], pts[-1][1], 3.0, fill=P.BLUE)
    svg.line(56, top + size - 20, 56 + scale * 1.0, top + size - 20, stroke=P.TEXT, width=1.5)
    svg.text(56, top + size - 28, "1 cm", size=10.5)
    svg.text(40 + size - 10, top + size - 12,
             "range %.1f cm; starts at r = %.2f cm" % (path, math.sqrt(2.0**2 + 2 * 2.0 * ELECTRON)
                                                      / 4.5), size=10.5, anchor="end",
             fill=P.MUTED)

    track2, seconds = process_track()
    left2 = 520
    pts2, scale2, _ = fit(track2, left2, top, size)
    svg.rect(left2, top, size, size, stroke=P.AXIS, width=1.0)
    svg.text(left2, top - 9, "A process track as specified (q > 0, clockwise)", size=13,
             weight="600")
    svg.polyline(pts2, stroke="#e6edf3", width=1.4)
    svg.circle(pts2[0][0], pts2[0][1], 3.0, fill=P.ORANGE)
    svg.circle(pts2[-1][0], pts2[-1][1], 3.0, fill=P.BLUE)
    hx, hy = fit([(0.0, 0.0)] + track2, left2, top, size)[0][0]
    svg.circle(hx, hy, 3.0, fill=P.MUTED)
    svg.line(left2 + 16, top + size - 20, left2 + 16 + scale2 * 5.0, top + size - 20,
             stroke=P.TEXT, width=1.5)
    svg.text(left2 + 16, top + size - 28, "5 world units", size=10.5)
    svg.text(left2 + size - 10, top + size - 12, "grey dot: home. CPU = 0.05 + 1.6 exp(-t / 14 s), "
             "track ends below 0.10", size=10.5, anchor="end", fill=P.MUTED)
    svg.save("docs/media/figures/chamber-spirals.svg")
    print("delta ray path cm", round(path, 3), "points", len(track))
    print("process track seconds", round(seconds, 2), "points", len(track2))


if __name__ == "__main__":
    main()
