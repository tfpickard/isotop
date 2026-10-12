"""Direction dependence of the lattice phase speed: five-point against nine-point stencil.

Writes docs/media/figures/wave-anisotropy.svg. Run from the repository root:
    python3 docs/figures/wave-anisotropy.py
Stdlib only, deterministic.
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w4 import *  # noqa: E402,F401,F403

NU = 0.5


def s5(kx, ky):
    return 4 - 2 * math.cos(kx) - 2 * math.cos(ky)


def s9(kx, ky):
    return (20 - 8 * math.cos(kx) - 8 * math.cos(ky) - 4 * math.cos(kx) * math.cos(ky)) / 6


def ratio(symbol, wavelength, theta):
    k = 2 * math.pi / wavelength
    kx, ky = k * math.cos(theta), k * math.sin(theta)
    omega = (2 / NU) * math.asin(0.5 * NU * math.sqrt(symbol(kx, ky)))
    return omega / k


figure = Figure(
    1100, 470,
    "Direction dependence of the lattice phase speed",
    "Left: phase speed relative to the continuum against propagation angle for the five-point and "
    "nine-point Laplacians. Right: the spread between the axis and diagonal directions against "
    "wavelength, which falls as the second power of the wavenumber for five points and the fourth for nine.",
)
figure.heading("Why nine points: the error is the same in every direction",
               "Courant number 0.5. Angle 0 is along a grid axis, 45 degrees along a diagonal.")

angles = [i * 90 / 90 for i in range(91)]
a = Axes(figure, 70, 90, 450, 300, (0, 90), (-4.0, 0.2), xlabel="propagation angle (degrees from a grid axis)",
         ylabel="phase speed error (percent)", title="Phase speed against direction",
         xticks=[0, 15, 30, 45, 60, 75, 90], yticks=[-4, -3, -2, -1, 0])
a.hline(0.0, MUTED, 1.0, "2 3")
for wavelength, dash in ((6, None), (8, "6 3"), (12, "2 2")):
    for symbol, color in ((s5, BLUE), (s9, ORANGE)):
        ys = [(ratio(symbol, wavelength, math.radians(t)) - 1) * 100 for t in angles]
        a.plot(angles, ys, color, 2.0, dash)
    a.label(91, (ratio(s9, wavelength, math.pi / 4) - 1) * 100, "%d cells" % wavelength, 11, "start", MUTED, dx=3, dy=4)
a.legend([("5-point", BLUE, None), ("9-point", ORANGE, None)], "bl")
a.label(46, -2.6, "9-point: flat", 11, "start", ORANGE)
a.label(46, -0.9, "5-point: slow along axes,", 11, "start", BLUE, dy=0)
a.label(46, -1.15, "fast along diagonals", 11, "start", BLUE, dy=0)

wavelengths = [3.5 * (60 / 3.5) ** (i / 120) for i in range(121)]
b = Axes(figure, 620, 90, 440, 300, (3.5, 60), (1e-6, 1e-1), xlog=True, ylog=True,
         xlabel="wavelength (cells)", ylabel="axis minus diagonal speed (percent of c)",
         title="Spread between directions", xticks=[4, 6, 8, 12, 20, 40, 60],
         yticks=[1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1], yminor=False)
for symbol, color in ((s5, BLUE), (s9, ORANGE)):
    ys = [abs(ratio(symbol, w, 0.0) - ratio(symbol, w, math.pi / 4)) * 100 for w in wavelengths]
    b.plot(wavelengths, ys, color, 2.0)
# guide slopes through the long-wave end
for power, color, label in ((2, BLUE, "slope 2 in k"), (4, ORANGE, "slope 4 in k")):
    k0 = 2 * math.pi / 60
    base = abs(ratio(s5 if power == 2 else s9, 60, 0.0) - ratio(s5 if power == 2 else s9, 60, math.pi / 4)) * 100
    ys = [base * (60 / w) ** power for w in wavelengths]
    b.plot(wavelengths, ys, color, 1.0, "3 3", opacity=0.8)
b.label(5.0, 1.6e-2, "5-point", 11, "start", BLUE)
b.label(9.0, 1.2e-4, "9-point", 11, "start", ORANGE)
b.vband(11.19, 42.0, AQUA, 0.16)
b.label(21.0, 2.0e-6, "pebble band", 11, "middle", AQUA)

figure.text(70, 436, "Dashed guides are k^2 and k^4 through the 60-cell end. Along an axis both stencils have the same symbol (2 - 2 cos k); only the diagonal differs.",
            11, "start", MUTED)
figure.text(70, 454, "Taylor series: 5-point symbol = k^2 - (kx^4 + ky^4)/12; 9-point symbol = k^2 - k^4/12, isotropic to fourth order.",
            11, "start", MUTED)
figure.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures", "wave-anisotropy.svg"))
