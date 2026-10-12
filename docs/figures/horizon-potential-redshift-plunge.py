"""Three computations behind the horizon view (G = c = M = 1).

Writes docs/media/figures/horizon-potential-redshift-plunge.svg. Run from the repository root:

    python3 docs/figures/horizon-potential-redshift-plunge.py

Left: the effective potential of a massive particle, V = (1 - 2/r)(1 + L^2/r^2), for several
angular momenta; circular orbits sit at its extrema, the marginally bound orbit (r = 4, E = 1)
and the innermost stable circular orbit (r = 6, an inflection point, L^2 = 12) are marked.
Middle: the redshift factor g = 1 / (1 + z) of a ring of disk gas at inclination 80 degrees,
1 + z = (1 - 3/r)^(-1/2) (1 + r^(-3/2) sin i x), with the screen coordinate x = r cos(phi) and no
light bending, and the relative intensity g^4. Right: a plunge from the ISCO with L = 0.95 L_isco
and an inward kick of 0.1, integrated in proper time: ln(r - 2) against Schwarzschild time t
(slope -1/2 asymptotically) and against the observer's retarded time u = t - r* (slope -1/4).
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402


def effective_potential(r, l_squared):
    return (1.0 - 2.0 / r) * (1.0 + l_squared / (r * r))


def redshift(r, phi, inclination=math.radians(80.0)):
    x = r * math.cos(phi)
    return (1.0 - 3.0 / r) ** -0.5 * (1.0 + r ** -1.5 * math.sin(inclination) * x)


def plunge(l_fraction=0.95, kick=0.1, t_max=60.0):
    l_value = l_fraction * math.sqrt(12.0)
    r = 6.0
    energy = math.sqrt((1 - 2 / r) * (1 + l_value**2 / r**2) + kick * kick)
    h, t = 0.01, 0.0
    track = [(0.0, r)]

    def drdtau(rr):
        v = energy**2 - (1 - 2 / rr) * (1 + l_value**2 / rr**2)
        return -math.sqrt(max(v, 1e-12))

    while t < t_max:
        k1 = drdtau(r)
        k2 = drdtau(r + 0.5 * h * k1)
        k3 = drdtau(r + 0.5 * h * k2)
        k4 = drdtau(r + h * k3)
        new = r + h / 6 * (k1 + 2 * k2 + 2 * k3 + k4)
        mid = 0.5 * (r + new)
        t += h * energy / (1 - 2 / mid)
        r = new
        if r < 2 + 1e-9:
            break
        track.append((t, r))
        h = min(0.01, 0.02 * (r - 2))
    return track


def main():
    svg = P.Svg(1240, 450, "Effective potential, disk redshift and plunge",
                "Effective potential of a massive particle around a Schwarzschild black hole, "
                "the redshift and intensity of a ring of disk gas, and the exponential approach "
                "of a plunging body to the horizon.")
    svg.text(24, 28, "Schwarzschild, G = c = M = 1: potential, Doppler beaming, plunge", size=16,
             weight="600")
    svg.text(24, 47, "Left: orbits sit at extrema of V; the ISCO is the inflection at r = 6. "
             "Middle: the approaching side is brighter by g^4. Right: the plunge clock matters "
             "by a factor of two.", size=11, fill=P.MUTED)
    top, h, w = 100, 290, 340

    radii = [2.02 + k * 0.05 for k in range(0, 321)]
    axes = P.Axes(svg, 60, top, w, h, (2, 18), (0.7, 1.05), xlabel="radius r (M)",
                  ylabel="effective potential V(r)", title="V = (1 - 2/r)(1 + L^2/r^2)",
                  xticks=[2, 4, 6, 8, 10, 12, 14, 16, 18], yticks=[0.7, 0.8, 0.9, 1.0])
    for l_squared, colour, name in ((12.0, P.YELLOW, "L^2 = 12 (ISCO)"), (13.0, P.AQUA, "13"),
                                    (16.0, P.ORANGE, "16 (E = 1 at r = 4)"),
                                    (20.0, P.VIOLET, "20")):
        axes.plot(radii, [effective_potential(r, l_squared) for r in radii], stroke=colour,
                  width=2.0)
    for radius, text in ((3.0, "photon sphere"), (4.0, "marginally bound"), (6.0, "ISCO")):
        axes.vline(radius, dash="2 4", opacity=0.6)
    svg.legend(250, top + 22, [("L^2 = 12 (ISCO)", P.YELLOW, "line"), ("L^2 = 13", P.AQUA, "line"),
                               ("L^2 = 16 (marginally bound)", P.ORANGE, "line"),
                               ("L^2 = 20", P.VIOLET, "line")], size=10.5, gap=15)
    axes.label(3.0, 0.715, "3", size=10.5, fill=P.MUTED, anchor="middle")
    axes.label(4.0, 0.715, "4", size=10.5, fill=P.MUTED, anchor="middle")
    axes.label(6.0, 0.715, "6", size=10.5, fill=P.MUTED, anchor="middle")

    phis = [k * math.pi / 90 for k in range(0, 181)]
    axes2 = P.Axes(svg, 480, top, w, h, (0, 180), (0.01, 10), ylog=True,
                   xlabel="azimuth phi around the ring (deg); 0 = receding, 180 = approaching",
                   ylabel="g^4 = (1 / (1 + z))^4", title="Doppler beaming, i = 80 degrees",
                   xticks=[0, 45, 90, 135, 180], yticks=[0.01, 0.1, 1, 10])
    degrees = [math.degrees(p) for p in phis]
    for radius, colour in ((6.0, P.ORANGE), (10.0, P.AQUA), (20.0, P.BLUE)):
        axes2.plot(degrees, [redshift(radius, p) ** -4 for p in phis], stroke=colour, width=2.0)
    axes2.hline(1.0, dash="2 4")
    svg.legend(600, top + 22, [("r = 6 M", P.ORANGE, "line"), ("r = 10 M", P.AQUA, "line"),
                               ("r = 20 M", P.BLUE, "line")], size=10.5, gap=15)

    track = plunge()
    ts = [t for t, r in track if r - 2 > 1e-6]
    rs = [r for t, r in track if r - 2 > 1e-6]
    observer = [t - (r + 2 * math.log(r / 2 - 1)) for t, r in zip(ts, rs)]
    logs = [math.log(r - 2) for r in rs]
    axes3 = P.Axes(svg, 890, top, w, h, (0, 60), (-12, 2), xlabel="time (M)",
                   ylabel="ln(r - 2M)", title="Plunge from the ISCO",
                   xticks=[0, 10, 20, 30, 40, 50, 60], yticks=[-12, -9, -6, -3, 0])
    axes3.plot(ts, logs, stroke=P.ORANGE, width=2.0)
    axes3.plot([max(o, 0.0) for o in observer], logs, stroke=P.BLUE, width=2.0, dash="6 4")
    svg.legend(1000, top + h - 52, [("coordinate time t, slope -1/2", P.ORANGE, "line"),
                                    ("observer's time u = t - r*, slope -1/4", P.BLUE, "dash")],
               size=10.5, gap=16)
    svg.save("docs/media/figures/horizon-potential-redshift-plunge.svg")

    for radius in (6.0, 10.0, 20.0):
        approaching = redshift(radius, math.pi) ** -4
        receding = redshift(radius, 0.0) ** -4
        print("r", radius, "g at phi=180", round(1 / redshift(radius, math.pi), 3),
              "g at phi=0", round(1 / redshift(radius, 0.0), 3), "g^4 ratio",
              round(approaching / receding, 1))


if __name__ == "__main__":
    main()
