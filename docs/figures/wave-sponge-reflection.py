"""The pond's absorbing border: damping profile and measured reflection at normal incidence.

Writes docs/media/figures/wave-sponge-reflection.svg. Run from the repository root:
    python3 docs/figures/wave-sponge-reflection.py
The reflection is measured with the exact scheme of medium.rs (the nine-point stencil reduces to
three points for an axis-aligned plane wave): a Ricker pulse travels towards the border, and the
reflected part is the difference between that run and one on a domain 600 cells longer.
Stdlib only, deterministic.
"""
import cmath
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w4 import *  # noqa: E402,F401,F403

C = 14.0
DT = 0.5 / C  # Courant number 0.5, isotop's largest
GAMMA0, SPONGE, WIDTH, KAPPA = 0.35, 6.0, 10.0, 0.5


def damping(edge):
    depth = max((WIDTH - edge) / WIDTH, 0.0)
    return GAMMA0 + SPONGE * depth * depth


def run(cells, extra, x0=150.0, probe=10, steps=1260, centre=0.8):
    total = cells + extra
    gamma = [damping(min(i, total - 1 - i)) for i in range(total)]
    a = math.pi * centre / C

    def ricker(x):
        u = a * x
        return (1 - 2 * u * u) * math.exp(-u * u)

    def slope(x):
        u = a * x
        return a * (-4 * u) * math.exp(-u * u) + (1 - 2 * u * u) * math.exp(-u * u) * (-2 * u * a)

    height = [ricker(i - extra - x0) for i in range(total)]
    velocity = [C * slope(i - extra - x0) for i in range(total)]  # left-moving
    record = []
    at = probe + extra
    for _ in range(steps):
        for i in range(1, total - 1):
            laplacian = height[i - 1] - 2 * height[i] + height[i + 1]
            velocity[i] += (C * C * laplacian - gamma[i] * velocity[i] - KAPPA * height[i]) * DT
        for i in range(total):
            height[i] += velocity[i] * DT
        record.append(height[at])
    return record


with_border = run(600, 0)
without = run(600, 600)
reflected = [x - y for x, y in zip(with_border, without)]


def spectrum(signal, frequency):
    return abs(sum(v * cmath.exp(-2j * math.pi * frequency * k * DT) for k, v in enumerate(signal)))


frequencies = [0.2 + 1.8 * i / 36 for i in range(37)]
coefficient = [spectrum(reflected, f) / spectrum(without, f) for f in frequencies]
round_trip = math.exp(-(WIDTH / C) * (GAMMA0 + SPONGE / 3.0))

figure = Figure(
    1100, 455,
    "The pond's absorbing border",
    "Left: the damping rate against distance from the border, 0.35 per second plus a quadratic "
    "sponge of 6 per second over ten cells. Right: the amplitude reflection coefficient measured "
    "with the exact scheme at normal incidence, between 0.46 at 0.35 hertz and 0.18 at 0.8 hertz.",
)
figure.heading("A sponge, not a perfect absorber",
               "Damping profile of Wave::new and the reflection it leaves at normal incidence (measured with the same scheme).")

edges = [i * 0.1 for i in range(0, 241)]
a = Axes(figure, 70, 90, 400, 270, (0, 24), (0, 7), xlabel="distance from the border (cells)",
         ylabel="damping rate gamma (1/s)", title="Damping profile", xticks=[0, 5, 10, 15, 20],
         yticks=[0, 1, 2, 3, 4, 5, 6, 7])
a.plot(edges, [damping(e) for e in edges], BLUE, 2.2)
a.hline(GAMMA0, MUTED, 1.0, "4 3", "background 0.35", 12.5)
a.vline(10, MUTED, 1.0, "4 3")
a.label(10.4, 5.6, "sponge ends at 10 cells", 11, "start", MUTED)
a.label(0.5, 6.7, "frame (never updated)", 11, "start", MUTED)

b = Axes(figure, 620, 90, 440, 270, (0.2, 2.0), (0, 0.8), xlabel="frequency (Hz)",
         ylabel="amplitude reflection coefficient", title="Reflection at normal incidence",
         xticks=[0.2, 0.5, 1.0, 1.5, 2.0], yticks=[0, 0.2, 0.4, 0.6, 0.8])
b.vband(0.35, 1.25, AQUA, 0.16)
b.label(0.8, 0.05, "pebble pitches", 11, "middle", AQUA)
b.plot(frequencies, coefficient, ORANGE, 2.2)
b.scatter(frequencies, coefficient, ORANGE, 2.5)
b.hline(round_trip, MUTED, 1.0, "4 3", "geometric-optics estimate %.2f" % round_trip, 1.1)

figure.text(70, 428, "Measured: 0.46 at 0.35 Hz, 0.31 at 0.5 Hz, 0.18 at 0.8 Hz. At high pitch the layer simply attenuates (round trip exp(-(10/c)(0.35 + 2)) = %.2f);" % round_trip,
            11, "start", MUTED)
figure.text(70, 444, "at low pitch the damping changes too fast for the wavelength and the layer reflects like a partial mirror.", 11, "start", MUTED)
figure.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures", "wave-sponge-reflection.svg"))
