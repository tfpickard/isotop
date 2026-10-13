"""Why the default Flow step spirals small, busy vortices outwards.

Writes docs/media/figures/flow-vortex-integrators.svg. Run from the repository root:
    python3 docs/figures/flow-vortex-integrators.py
Left: the modulus of the one-step stability function on rotation, zeta = Omega dt, for explicit
Euler, the explicit midpoint (RK2) method that Flow::advance uses, classical RK4 and the implicit
midpoint rule; the dots are the three example processes of the page at dt = 0.1. Right: radius of
a tracer started at 0.94 units in the core of the 6 MiB, 400% CPU vortex (Gamma 60.5, core 1.87,
speed capped at 20 as in Scene::weather) against time, for several steps.
Stdlib only, deterministic.
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w4 import *  # noqa: E402,F401,F403

GAMMA, CORE = 60.5, 1.87
CAP = 20.0


def field(point):
    x, y = point
    swirl = GAMMA / (x * x + y * y + CORE * CORE)
    vx, vy = -y * swirl, x * swirl
    speed = math.hypot(vx, vy)
    if speed > CAP:
        vx, vy = vx * CAP / speed, vy * CAP / speed
    return vx, vy


def midpoint(point, dt):
    k1 = field(point)
    k2 = field((point[0] + 0.5 * dt * k1[0], point[1] + 0.5 * dt * k1[1]))
    return point[0] + dt * k2[0], point[1] + dt * k2[1]


def radius_history(dt, seconds=5.0):
    point = (0.94, 0.0)
    times, radii = [0.0], [math.hypot(*point)]
    steps = int(round(seconds / dt))
    for n in range(1, steps + 1):
        point = midpoint(point, dt)
        times.append(n * dt)
        radii.append(math.hypot(*point))
    return times, radii


figure = Figure(
    1160, 495,
    "RK2 midpoint on a vortex",
    "Left: per-step growth of the radius of a rotating tracer against the angular step, for four "
    "integrators; the explicit midpoint method used by Flow always grows. Right: a tracer started "
    "inside a small, busy vortex is thrown out to a radius near 2.7 at the default 0.1 second step.",
)
figure.heading("The step Flow takes against the rotation it integrates",
               "Rotation z' = i Omega z, zeta = Omega dt. The exact map has modulus 1. Default 10 fps means dt = 0.1 s.")

zetas = [i * 3.0 / 300 for i in range(301)]
a = Axes(figure, 70, 90, 480, 300, (0, 3.0), (0.4, 3.4), xlabel="angular step zeta = Omega * dt",
         ylabel="radius multiplier per step", title="Growth of the radius per step",
         xticks=[0, 0.5, 1, 1.5, 2, 2.5, 3], yticks=[0.5, 1, 1.5, 2, 2.5, 3])
a.hline(1.0, MUTED, 1.0, "2 3")
a.plot(zetas, [math.sqrt(1 + z * z) for z in zetas], BLUE, 2.0, "6 3")
a.plot(zetas, [math.sqrt(1 + z ** 4 / 4) for z in zetas], ORANGE, 2.6)
a.plot(zetas, [abs(1 + 1j * z - z * z / 2 - 1j * z ** 3 / 6 + z ** 4 / 24) for z in zetas], AQUA, 2.0)
a.plot(zetas, [1.0 for z in zetas], VIOLET, 2.0, "2 2")
a.legend([("explicit Euler", BLUE, "6 3"), ("explicit midpoint (Flow)", ORANGE, None),
          ("RK4", AQUA, None), ("implicit midpoint", VIOLET, "2 2")], "tl")
for z, label, dx, dy in ((1.73, "6 MiB at 400%", -6, -10), (0.46, "64 MiB at 50%", 10, 18), (0.14, "1 GiB", 10, -14)):
    a.scatter([z], [math.sqrt(1 + z ** 4 / 4)], YELLOW, 4.5)
    a.label(z, math.sqrt(1 + z ** 4 / 4), label, 11, "start" if dx > 0 else "end", INK, dx=dx, dy=dy)

b = Axes(figure, 640, 90, 480, 300, (0, 5), (0, 3.6), xlabel="time (s)", ylabel="tracer radius (world units)",
         title="A tracer in the 6 MiB, 400% vortex", xticks=[0, 1, 2, 3, 4, 5], yticks=[0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5])
b.hline(0.94, MUTED, 1.0, "2 3", "exact: stays at 0.94", 3.6, "start")
for dt, color, dash in ((0.1, ORANGE, None), (0.05, YELLOW, None), (1 / 30, AQUA, None), (0.01, BLUE, "6 3")):
    times, radii = radius_history(dt)
    b.plot(times, radii, color, 2.0, dash)
b.legend([("dt = 0.1 s (10 fps)", ORANGE, None), ("dt = 0.05 s", YELLOW, None),
          ("dt = 1/30 s (30 fps)", AQUA, None), ("dt = 0.01 s", BLUE, "6 3")], "tl")
figure.text(70, 452, "Circles are the conserved level lines of the stream function: a good integrator keeps tracers on them. Midpoint grows as zeta^4/8 per step for small zeta.", 11, "start", MUTED)
figure.text(70, 470, "Analytic field with the 20 units per second speed cap; Flow samples a cached grid by bilinear interpolation, which changes the numbers a little.", 11, "start", MUTED)
figure.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures", "flow-vortex-integrators.svg"))
