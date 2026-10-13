"""Ionisation density of a charged particle in liquid hydrogen, and isotop's 1/beta^2.

Writes docs/media/figures/chamber-bethe.svg. Run from the repository root:

    python3 docs/figures/chamber-bethe.py

Mean ionisation energy loss by the Bethe formula for a heavy charged particle of charge e,

    -dE/dx = K (Z/A) rho / beta^2 [ 1/2 ln(2 m_e c^2 beta^2 gamma^2 T_max / I^2) - beta^2 ],

with K = 0.307075 MeV cm^2 / mol, Z/A = 0.99212 mol/g, I = 21.8 eV, rho = 0.0708 g/cm^3 for
liquid hydrogen, and T_max = 2 m_e c^2 beta^2 gamma^2 / (1 + 2 gamma m_e/M + (m_e/M)^2). The
density-effect and shell corrections are left out, which is adequate for hydrogen. The number of
bubbles per unit length follows the ionisation, not the mean loss itself; this figure plots the
mean loss.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

K = 0.307075
ZA = 0.99212
IONISATION = 21.8e-6
ELECTRON = 0.510999
DENSITY = 0.0708
MASSES = {"pion": 139.57, "kaon": 493.68, "proton": 938.27}


def bethe(beta_gamma, mass):
    gamma = math.sqrt(1.0 + beta_gamma**2)
    beta2 = beta_gamma**2 / gamma**2
    t_max = 2 * ELECTRON * beta_gamma**2 / (1 + 2 * gamma * ELECTRON / mass
                                            + (ELECTRON / mass) ** 2)
    return K * ZA / beta2 * (0.5 * math.log(2 * ELECTRON * beta_gamma**2 * t_max
                                            / IONISATION**2) - beta2)


def main():
    grid = [10 ** (k / 100.0 - 1.0) for k in range(0, 501)]
    minimum = min(bethe(x, MASSES["pion"]) for x in grid)
    svg = P.Svg(1120, 440, "Ionisation loss in liquid hydrogen against 1/beta^2",
                "Bethe-Bloch mean energy loss relative to its minimum against beta gamma, "
                "compared with the clamped 1/beta^2 of the chamber view; and the loss against "
                "momentum for pion, kaon and proton.")
    svg.text(24, 28, "Ionisation in liquid hydrogen: what 1/beta^2 keeps and what it drops",
             size=16, weight="600")
    svg.text(24, 47, "Bethe formula without density correction. Minimum ionisation here: "
             "%.2f MeV cm^2/g at beta gamma = 3.5, %.3f MeV/cm in liquid hydrogen."
             % (minimum, minimum * DENSITY), size=11, fill=P.MUTED)
    top, h = 100, 290
    axes = P.Axes(svg, 70, top, 480, h, (0.1, 1000), (0.5, 40), xlog=True, ylog=True,
                  xlabel="beta gamma = p / (m c)", ylabel="loss relative to minimum ionisation",
                  title="Mean loss, any charge-1 particle",
                  xticks=[0.1, 1, 10, 100, 1000], yticks=[0.5, 1, 2, 4, 8, 16, 32],
                  xtick_labels=["0.1", "1", "10", "100", "1000"],
                  ytick_labels=["0.5", "1", "2", "4", "8", "16", "32"])
    axes.plot(grid, [bethe(x, MASSES["pion"]) / minimum for x in grid], stroke=P.BLUE,
              width=2.4)

    def inverse_beta2(x):
        return min(1.0 + 1.0 / x**2, 8.0)

    axes.plot(grid, [inverse_beta2(x) for x in grid], stroke=P.ORANGE, width=2.0, dash="6 4")
    axes.hline(8, dash="2 4")
    svg.legend(300, top + 28, [("Bethe formula (pion)", P.BLUE, "line"),
                               ("min(1 / beta^2, 8), the view's density", P.ORANGE, "dash")],
               size=11, gap=17)
    axes.label(0.12, 8.6, "clamp 8", size=10.5, fill=P.MUTED)
    axes.label(3.5, 0.62, "minimum at beta gamma = 3.5", size=10.5, fill=P.MUTED,
               anchor="middle")

    # Second panel: at fixed momentum, mass sets the loss.
    momenta = [10 ** (k / 100.0 - 1.0) for k in range(0, 301)]
    axes2 = P.Axes(svg, 640, top, 440, h, (0.1, 100), (0.2, 100), xlog=True, ylog=True,
                   xlabel="momentum p (GeV/c)", ylabel="mean loss (MeV/cm)",
                   title="At fixed momentum, heavier means denser",
                   xticks=[0.1, 1, 10, 100], yticks=[0.2, 0.5, 1, 2, 5, 10, 20, 50, 100],
                   xtick_labels=["0.1", "1", "10", "100"],
                   ytick_labels=["0.2", "0.5", "1", "2", "5", "10", "20", "50", "100"])
    for (name, mass), colour in zip(MASSES.items(), (P.BLUE, P.AQUA, P.ORANGE)):
        xs, ys = [], []
        for p in momenta:
            beta_gamma = p * 1000.0 / mass
            xs.append(p)
            ys.append(bethe(beta_gamma, mass) * DENSITY)
        axes2.plot(xs, ys, stroke=colour, width=2.2)
    svg.legend(940, top + 30, [("pion", P.BLUE, "line"), ("kaon", P.AQUA, "line"),
                                   ("proton", P.ORANGE, "line")], size=11, gap=17)
    svg.save("docs/media/figures/chamber-bethe.svg")
    print("minimum MeV cm2/g", round(minimum, 4), "MeV/cm", round(minimum * DENSITY, 4))
    for bg in (0.38, 1.0, 3.5, 100.0):
        print("beta gamma", bg, "bethe/min", round(bethe(bg, MASSES["pion"]) / minimum, 3),
              "min(1+1/bg^2, 8)", round(inverse_beta2(bg), 3))
    for p in (0.5, 1.0):
        row = {name: round(bethe(p * 1000 / m, m) * DENSITY, 3) for name, m in MASSES.items()}
        print("p", p, "GeV/c MeV/cm", row)


if __name__ == "__main__":
    main()
