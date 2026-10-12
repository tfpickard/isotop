"""Light bending by a Schwarzschild black hole, from the orbit equation d2u/dphi2 + u = 3 M u^2.

Writes docs/media/figures/schwarzschild-light-bending.svg. Run from the repository root:

    python3 docs/figures/schwarzschild-light-bending.py

With G = c = M = 1 and u = 1/r, a photon of impact parameter b obeys u'' = 3 u^2 - u with
u(0) = 0 and u'(0) = 1/b, integrated here with the classical fourth-order Runge-Kutta method
(step 0.002 rad). A ray with b < b_c = 3 sqrt(3) falls in; one with b > b_c is deflected by
alpha = (total sweep) - pi. Left: rays from the left in the orbital plane. Right: alpha against b,
with the weak-field expansion 4/b + (15 pi / 4) / b^2 and the strong-field logarithm
-ln(b / b_c - 1) + ln(216 (7 - 4 sqrt 3)) - pi.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

B_CRITICAL = 3.0 * math.sqrt(3.0)
STEP = 0.002


def trace(b, limit=40.0, keep_path=False):
    """Returns (captured, total_sweep, path of (phi, u))."""
    u, v, phi = 0.0, 1.0 / b, 0.0
    path = [(0.0, 0.0)] if keep_path else None

    def acceleration(x):
        return 3.0 * x * x - x

    while phi < limit:
        k1u, k1v = v, acceleration(u)
        k2u, k2v = v + 0.5 * STEP * k1v, acceleration(u + 0.5 * STEP * k1u)
        k3u, k3v = v + 0.5 * STEP * k2v, acceleration(u + 0.5 * STEP * k2u)
        k4u, k4v = v + STEP * k3v, acceleration(u + STEP * k3u)
        previous = u
        u += STEP / 6 * (k1u + 2 * k2u + 2 * k3u + k4u)
        v += STEP / 6 * (k1v + 2 * k2v + 2 * k3v + k4v)
        phi += STEP
        if keep_path:
            path.append((phi, u))
        if u >= 0.5:
            return True, phi, path
        if u <= 0.0:
            # the ray has left: interpolate the crossing of u = 0 inside the last step
            phi -= STEP * (-u) / (previous - u)
            return False, phi, path
    return False, phi, path


def main():
    svg = P.Svg(1180, 470, "Light bending by a Schwarzschild black hole",
                "Photon paths for several impact parameters around a Schwarzschild black hole "
                "and the deflection angle against impact parameter.")
    svg.text(24, 28, "Photons around a Schwarzschild hole: u'' + u = 3 M u^2", size=16,
             weight="600")
    svg.text(24, 47, "Impact parameter b in units of M; shadow radius b_c = 3 sqrt(3) = 5.196. "
             "Rays with b < b_c are captured.", size=11, fill=P.MUTED)

    left, top, size = 40, 100, 360
    span = 16.0
    scale = size / (2 * span)
    svg.rect(left, top, size, size, stroke=P.AXIS, width=1.0)
    clip = svg.clip_rect(left, top, size, size)
    cx, cy = left + size / 2, top + size / 2
    svg.circle(cx, cy, B_CRITICAL * scale, stroke=P.MUTED, width=1.0, dash="4 3")
    svg.circle(cx, cy, 3.0 * scale, stroke=P.MUTED, width=1.0, dash="1 3")
    svg.circle(cx, cy, 2.0 * scale, fill="#05070a", stroke=P.TEXT, width=1.2)
    rays = [(3.0, P.RED), (5.0, P.RED), (5.25, P.ORANGE), (5.6, P.YELLOW), (6.5, P.AQUA),
            (9.0, P.BLUE), (14.0, P.VIOLET)]
    for b, colour in rays:
        captured, sweep, path = trace(b, keep_path=True)
        points = []
        for phi, u in path:
            if u <= 1e-4:
                continue
            r = 1.0 / u
            if r > 60:
                r = 60.0
            points.append((cx + (-r * math.cos(phi)) * scale, cy - (r * math.sin(phi)) * scale))
        svg.polyline(points, stroke=colour, width=1.8, clip=clip)
        label = "b = %g" % b
        svg.text(left + 6, cy - b * scale - 4 if b < 15 else top + 12, label, size=10.5,
                 fill=colour)
    svg.text(cx + 3.0 * scale + 3, cy - 4, "photon sphere 3M", size=10, fill=P.MUTED)
    svg.text(left + 8, top + size - 10, "dashed: shadow edge b_c; dotted: photon sphere; "
             "filled: horizon 2M", size=10, fill=P.MUTED)

    axes = P.Axes(svg, 560, top, 560, size - 20, (4, 40), (0.05, 20), xlog=True, ylog=True,
                  xlabel="impact parameter b (M)", ylabel="deflection angle alpha (rad)",
                  title="Deflection angle", xticks=[4, 6, 8, 10, 20, 40],
                  yticks=[0.05, 0.1, 0.2, 0.5, 1, 2, 5, 10, 20],
                  ytick_labels=["0.05", "0.1", "0.2", "0.5", "1", "2", "5", "10", "20"],
                  xtick_labels=["4", "6", "8", "10", "20", "40"])
    bs, alphas = [], []
    steps = 140
    for k in range(steps + 1):
        # dense sampling just above b_c, geometric spacing later
        excess = 10 ** (-3.0 + 3.0 * k / steps) * 40.0
        b = B_CRITICAL * (1.0 + 0.0) + excess if excess < 40 else 40.0
        b = B_CRITICAL + excess * (40.0 - B_CRITICAL) / 40.0
        captured, sweep, _ = trace(b, limit=60.0)
        if captured:
            continue
        bs.append(b)
        alphas.append(sweep - math.pi)
    axes.plot(bs, alphas, stroke=P.BLUE, width=2.4)
    weak = [4.0 + 36.0 * k / 200 for k in range(201)]
    axes.plot(weak, [4.0 / b for b in weak], stroke=P.MUTED, width=1.5, dash="6 4")
    axes.plot(weak, [4.0 / b + 15.0 * math.pi / 4.0 / b**2 for b in weak], stroke=P.AQUA,
              width=1.5, dash="6 4")
    constant = math.log(216.0 * (7.0 - 4.0 * math.sqrt(3.0))) - math.pi
    strong = [B_CRITICAL * (1.0 + 10 ** (-3 + 2.3 * k / 100)) for k in range(101)]
    axes.plot(strong, [-math.log(b / B_CRITICAL - 1.0) + constant for b in strong],
              stroke=P.ORANGE, width=1.5, dash="6 4")
    axes.vline(B_CRITICAL, dash="2 4")
    svg.legend(850, top + 24, [("numerical (RK4)", P.BLUE, "line"),
                               ("weak field 4M / b", P.MUTED, "dash"),
                               ("+ (15 pi / 4) (M / b)^2", P.AQUA, "dash"),
                               ("strong field, -ln(b / b_c - 1) %+.3f" % constant, P.ORANGE,
                                "dash")], size=10.5, gap=16)
    svg.save("docs/media/figures/schwarzschild-light-bending.svg")
    print("strong-field constant", round(constant, 4))
    for b in (6.0, 10.0, 20.0, 40.0):
        captured, sweep, _ = trace(b, limit=60.0)
        print("b", b, "alpha numerical", round(sweep - math.pi, 5), "4/b", round(4 / b, 5),
              "4/b + 15 pi/4 /b^2", round(4 / b + 15 * math.pi / 4 / b**2, 5),
              "strong", round(-math.log(b / B_CRITICAL - 1) + constant, 4))


if __name__ == "__main__":
    main()
