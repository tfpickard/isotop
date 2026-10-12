"""V0 decays: the neutral parent is invisible, its two charged daughters make a V.

Writes docs/media/figures/chamber-v0.svg. Run from the repository root:

    python3 docs/figures/chamber-v0.py

Left: a K0_S of 0.4 GeV/c decays to pi+ pi- at the origin, B = 1.5 T out of the page. The parent
leaves no track (dashed); the daughters curl in opposite senses. Middle: the Armenteros-
Podolanski plane (alpha, q_T) for K0_S -> pi+ pi-, Lambda -> p pi- and anti-Lambda -> pbar pi+
at a parent momentum of 2 GeV/c; alpha = (p_L+ - p_L-) / (p_L+ + p_L-), q_T is the daughter
momentum transverse to the parent. Right: the chance that a neutral parent has not yet decayed
after a distance L, exp(-L / (beta gamma c tau)), at 2 GeV/c.
Masses (MeV/c^2): K0 497.611, pi 139.570, Lambda 1115.683, proton 938.272. c tau: K0_S 2.684 cm,
Lambda 7.89 cm.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

M_K0, M_PI, M_LAMBDA, M_PROTON = 497.611, 139.570, 1115.683, 938.272
CTAU_K0S, CTAU_LAMBDA = 2.684, 7.89


def decay_momentum(parent, first, second):
    return math.sqrt((parent**2 - (first + second) ** 2) * (parent**2 - (first - second) ** 2)) \
        / (2.0 * parent)


def armenteros(parent, positive, negative, parent_momentum, samples=181):
    p_star = decay_momentum(parent, positive, negative)
    energy_p = (parent**2 + positive**2 - negative**2) / (2.0 * parent)
    energy_n = (parent**2 + negative**2 - positive**2) / (2.0 * parent)
    gamma_beta = parent_momentum / parent
    gamma = math.sqrt(1.0 + gamma_beta**2)
    beta = gamma_beta / gamma
    points = []
    for k in range(samples):
        theta = 2.0 * math.pi * k / (samples - 1)
        longitudinal_p = gamma * (p_star * math.cos(theta) + beta * energy_p)
        longitudinal_n = gamma * (-p_star * math.cos(theta) + beta * energy_n)
        alpha = (longitudinal_p - longitudinal_n) / (longitudinal_p + longitudinal_n)
        points.append((alpha, p_star * abs(math.sin(theta))))
    return points


def daughters(parent_mass, mass, parent_momentum, theta_star):
    """Lab momenta (p_x, p_y) of the positive and negative daughter of a symmetric decay."""
    p_star = decay_momentum(parent_mass, mass, mass)
    energy = parent_mass / 2.0
    gamma_beta = parent_momentum / parent_mass
    gamma = math.sqrt(1.0 + gamma_beta**2)
    beta = gamma_beta / gamma
    plus = (gamma * (p_star * math.cos(theta_star) + beta * energy), p_star * math.sin(theta_star))
    minus = (gamma * (-p_star * math.cos(theta_star) + beta * energy), -p_star * math.sin(theta_star))
    return plus, minus


def helix(start, momentum, charge, field, length_cm, step_cm=0.5):
    """Transverse-plane track of a particle with momentum (MeV/c) in a field of B tesla out of the
    page: positive charges turn clockwise."""
    px, py = momentum
    p = math.hypot(px, py)
    radius = p / (3.0 * field)
    theta = math.atan2(py, px)
    x, y = start
    points = [(x, y)]
    travelled = 0.0
    while travelled < length_cm:
        theta -= charge * step_cm / radius
        x += step_cm * math.cos(theta)
        y += step_cm * math.sin(theta)
        points.append((x, y))
        travelled += step_cm
    return points, radius


def main():
    svg = P.Svg(1180, 450, "V0 decays",
                "A neutral kaon decays into two curling pions; the Armenteros-Podolanski plane; "
                "the survival probability of a neutral parent.")
    svg.text(24, 28, "V0: a neutral parent that leaves no track and decays into a V", size=16,
             weight="600")
    svg.text(24, 47, "B = 1.5 T out of the page. In isotop's chamber a fork whose parent is "
             "neutral (below the CPU threshold) draws such a vertex.", size=11, fill=P.MUTED)
    top, h = 100, 290

    # Event display.
    left, w = 40, 330
    axes = P.Axes(svg, left, top, w, h, (-20, 130), (-66, 66), xlabel="x (cm)",
                  ylabel="y (cm)", title="K0_S (0.4 GeV/c) to pi+ pi-",
                  xticks=[0, 40, 80, 120], yticks=[-60, -30, 0, 30, 60])
    plus, minus = daughters(M_K0, M_PI, 400.0, math.radians(80.0))
    track_p, radius_p = helix((0.0, 0.0), plus, +1, 1.5, 120.0)
    track_n, radius_n = helix((0.0, 0.0), minus, -1, 1.5, 120.0)
    axes.plot([p[0] for p in track_p], [p[1] for p in track_p], stroke=P.BLUE, width=2.0)
    axes.plot([p[0] for p in track_n], [p[1] for p in track_n], stroke=P.ORANGE, width=2.0)
    axes.plot([-20, 0.0], [0.0, 0.0], stroke=P.MUTED, width=1.5, dash="5 4")
    axes.dots([0.0], [0.0], fill="#e6edf3", radius=3.5)
    axes.label(2, -8, "vertex", size=10.5, fill=P.MUTED, dx=4, dy=14)
    axes.label(-18, 9, "K0 (no track)", size=10.5, fill=P.MUTED)
    end_p, end_n = track_p[-1], track_n[-1]
    axes.label(end_p[0], end_p[1], "pi+", size=11, fill=P.BLUE, dx=-8, dy=18, anchor="end")
    axes.label(end_n[0], end_n[1], "pi-", size=11, fill=P.ORANGE, dx=-8, dy=-10, anchor="end")
    print("daughter radii cm", round(radius_p, 1), round(radius_n, 1))

    # Armenteros-Podolanski.
    parent_momentum = 2000.0
    axes2 = P.Axes(svg, 460, top, 330, h, (-1, 1), (0, 0.26), xlabel="alpha",
                   ylabel="q_T (GeV/c)", title="Armenteros-Podolanski, parent at 2 GeV/c",
                   xticks=[-1, -0.5, 0, 0.5, 1], yticks=[0, 0.05, 0.1, 0.15, 0.2, 0.25])
    curves = [("K0_S", M_K0, M_PI, M_PI, P.BLUE), ("Lambda", M_LAMBDA, M_PROTON, M_PI, P.ORANGE),
              ("anti-Lambda", M_LAMBDA, M_PI, M_PROTON, P.AQUA)]
    for name, parent, positive, negative, colour in curves:
        points = armenteros(parent, positive, negative, parent_momentum)
        axes2.plot([p[0] for p in points], [p[1] / 1000.0 for p in points], stroke=colour,
                   width=2.2)
    axes2.label(0.0, 0.222, "K0_S", anchor="middle", fill=P.BLUE, size=11)
    axes2.label(0.69, 0.115, "Lambda", anchor="middle", fill=P.ORANGE, size=11)
    axes2.label(-0.69, 0.115, "anti-Lambda", anchor="middle", fill=P.AQUA, size=11)

    # Survival.
    axes3 = P.Axes(svg, 850, top, 300, h, (0, 100), (0, 1.0), xlabel="distance from production L (cm)",
                   ylabel="probability not yet decayed", title="An invisible neutral, at 2 GeV/c",
                   xticks=[0, 25, 50, 75, 100], yticks=[0, 0.25, 0.5, 0.75, 1.0])
    xs = [k * 0.5 for k in range(0, 201)]
    for label, mass, ctau, colour in (("K0_S", M_K0, CTAU_K0S, P.BLUE),
                                      ("Lambda", M_LAMBDA, CTAU_LAMBDA, P.ORANGE)):
        mean = parent_momentum / mass * ctau
        axes3.plot(xs, [math.exp(-x / mean) for x in xs], stroke=colour, width=2.2)
        print(label, "mean decay length cm", round(mean, 2))
    svg.legend(960, top + 36, [("K0_S, mean decay length 10.8 cm", P.BLUE, "line"),
                               ("Lambda, mean decay length 14.1 cm", P.ORANGE, "line")],
               size=10.5, gap=16)
    svg.save("docs/media/figures/chamber-v0.svg")
    for name, parent, positive, negative, colour in curves:
        points = armenteros(parent, positive, negative, parent_momentum)
        print(name, "p* MeV/c", round(decay_momentum(parent, positive, negative), 1),
              "alpha range", round(min(p[0] for p in points), 3),
              round(max(p[0] for p in points), 3))


if __name__ == "__main__":
    main()
