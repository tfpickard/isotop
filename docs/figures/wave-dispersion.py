"""Lattice dispersion of the pond's wave scheme against the continuum.

Writes docs/media/figures/wave-dispersion.svg. Run from the repository root:
    python3 docs/figures/wave-dispersion.py
Stdlib only, deterministic.
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w4 import *  # noqa: E402,F401,F403

NU = 0.5  # Courant number c*dt/dx at its largest in isotop (medium.rs: substeps of half a cell)


def s5(kx, ky):
    return 4 - 2 * math.cos(kx) - 2 * math.cos(ky)


def s9(kx, ky):
    return (20 - 8 * math.cos(kx) - 8 * math.cos(ky) - 4 * math.cos(kx) * math.cos(ky)) / 6


def omega(symbol, nu, k, theta):
    """omega / c for a plane wave of wavenumber k (rad per cell) travelling at angle theta, from
    sin(omega dt / 2) = (c dt / 2) sqrt(symbol), with dt = nu / c and dx = 1 (undamped)."""
    kx, ky = k * math.cos(theta), k * math.sin(theta)
    return (2 / nu) * math.asin(0.5 * nu * math.sqrt(symbol(kx, ky)))


def phase_ratio(symbol, nu, k, theta):
    return omega(symbol, nu, k, theta) / k


def group_ratio(symbol, nu, k, theta):
    h = 1e-5 * k
    return (omega(symbol, nu, k + h, theta) - omega(symbol, nu, k - h, theta)) / (2 * h)


figure = Figure(
    1200, 440,
    "Lattice dispersion of the pond's wave scheme",
    "Phase speed, group speed and angular frequency of plane waves on the nine-point and five-point "
    "lattices at Courant number 0.5, against the continuum. Wavelengths of the pond's pebbles span "
    "11 to 42 cells, where the error is at most about one percent.",
)
figure.heading("Lattice dispersion at Courant number 0.5",
               "Plane waves on the discrete pond. The continuum is the line at 1 (left, middle) and omega = c k (right).")

wavelengths = [3 * (60 / 3) ** (i / 160) for i in range(161)]
curves = [
    ("5-point, along an axis", BLUE, None, s5, 0.0),
    ("5-point, along a diagonal", BLUE, "5 3", s5, math.pi / 4),
    ("9-point, along an axis", ORANGE, None, s9, 0.0),
    ("9-point, along a diagonal", ORANGE, "5 3", s9, math.pi / 4),
]

a = Axes(figure, 60, 90, 330, 280, (3, 60), (0.84, 1.02), xlog=True, xlabel="wavelength (cells)",
         ylabel="phase speed / c", title="Phase speed", xticks=[3, 4, 6, 8, 12, 20, 40, 60],
         yticks=[0.85, 0.9, 0.95, 1.0])
a.vband(11.19, 42.0, AQUA, 0.16)
a.label(16.5, 0.847, "pebble band", 11, "middle", AQUA)
a.hline(1.0, MUTED, 1.0, "2 3")
for name, color, dash, symbol, theta in curves:
    a.plot(wavelengths, [phase_ratio(symbol, NU, 2 * math.pi / w, theta) for w in wavelengths], color, 1.9, dash)
a.legend([(n, c, d) for n, c, d, _, _ in curves], "bl")

b = Axes(figure, 460, 90, 330, 280, (3, 60), (0.0, 1.05), xlog=True, xlabel="wavelength (cells)",
         ylabel="group speed / c", title="Group speed (energy travels at this)",
         xticks=[3, 4, 6, 8, 12, 20, 40, 60], yticks=[0, 0.25, 0.5, 0.75, 1.0])
b.vband(11.19, 42.0, AQUA, 0.16)
b.hline(1.0, MUTED, 1.0, "2 3")
for name, color, dash, symbol, theta in curves:
    b.plot(wavelengths, [group_ratio(symbol, NU, 2 * math.pi / w, theta) for w in wavelengths], color, 1.9, dash)
b.label(3.2, 0.12, "short waves lag: they carry energy", 11, "start", MUTED)
b.label(3.2, 0.04, "slower than the crest speed suggests", 11, "start", MUTED)

ks = [math.pi * i / 200 for i in range(1, 201)]
c = Axes(figure, 860, 90, 310, 280, (0, math.pi), (0, 3.2), xlabel="wavenumber k (rad per cell)",
         ylabel="omega / c (rad per cell)", title="Band diagram", xticks=[0, 0.5, 1, 1.5, 2, 2.5, 3],
         yticks=[0, 1, 2, 3])
c.vband(0.1495, 0.5613, AQUA, 0.22)
c.plot(ks, ks, MUTED, 1.4, "2 3")
c.plot(ks, [omega(s9, NU, k, 0.0) for k in ks], ORANGE, 2.0)
c.plot(ks, [omega(s9, NU, k, math.pi / 4) for k in ks], ORANGE, 2.0, "5 3")
c.plot(ks, [omega(s5, NU, k, 0.0) for k in ks], BLUE, 2.0)
c.label(0.62, 0.38, "pebbles", 11, "start", AQUA)
c.label(2.0, 2.0, "continuum", 11, "start", MUTED, dx=-6, dy=-18)
c.label(1.55, 1.58, "axis lattice flattens", 11, "start", ORANGE, dy=26)
c.label(1.55, 1.58, "toward the Nyquist limit k = pi", 11, "start", ORANGE, dy=40)

figure.text(60, 412, "Courant number 0.5 is isotop's ceiling (Wave::advance); the 9-point stencil is stable to sqrt(3)/2 = 0.866, the 5-point to 1/sqrt(2) = 0.707.",
            11, "start", MUTED)
figure.text(60, 428, "The Klein-Gordon term (0.5 per s squared) and the damping are left out here; see the source-band figure on View Ripple.",
            11, "start", MUTED)
figure.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures", "wave-dispersion.svg"))
