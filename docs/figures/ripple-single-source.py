"""One pebble at 100% CPU and 0.8 Hz on the Ripple pond, stepped with the exact scheme of medium.rs.

Writes docs/media/figures/ripple-single-source.svg. Run from the repository root:
    python3 docs/figures/ripple-single-source.py
Left: the height field after 9 s on a 151 x 85 grid (nine-point stencil, damping 0.35 plus the
ten-cell sponge, restoring term 0.5, frame steps of 0.1 s in three substeps as at --fps 10).
Right: the amplitude envelope along a ray against the Hankel-wave prediction
A r^(-1/2) exp(-alpha r), alpha = gamma omega / (2 c sqrt(omega^2 - kappa)).
Stdlib only, deterministic (about ten seconds).
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w4 import *  # noqa: E402,F401,F403

C, GAMMA0, SPONGE, WIDTH, KAPPA = 14.0, 0.35, 6.0, 10.0, 0.5
COLUMNS, ROWS = 151, 85
FREQUENCY = 0.8
OMEGA = 2 * math.pi * FREQUENCY
LOUDNESS = 900.0 * 100.0 / (100.0 + 40.0)
CENTRE = (75, 42)

damping = [0.0] * (COLUMNS * ROWS)
for y in range(ROWS):
    for x in range(COLUMNS):
        edge = min(x, y, COLUMNS - 1 - x, ROWS - 1 - y)
        depth = max((WIDTH - edge) / WIDTH, 0.0)
        damping[y * COLUMNS + x] = GAMMA0 + SPONGE * depth * depth
height = [0.0] * (COLUMNS * ROWS)
velocity = [0.0] * (COLUMNS * ROWS)


def drive(amount):
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            weight = (1.0, 0.5, 0.25)[abs(dx) + abs(dy)]
            velocity[(CENTRE[1] + dy) * COLUMNS + CENTRE[0] + dx] += amount * weight


def step(dt):
    c2 = C * C
    for y in range(1, ROWS - 1):
        base = y * COLUMNS
        for x in range(1, COLUMNS - 1):
            i = base + x
            laplacian = (4.0 * (height[i - 1] + height[i + 1] + height[i - COLUMNS] + height[i + COLUMNS])
                         + height[i - COLUMNS - 1] + height[i - COLUMNS + 1]
                         + height[i + COLUMNS - 1] + height[i + COLUMNS + 1] - 20.0 * height[i]) / 6.0
            velocity[i] += (c2 * laplacian - damping[i] * velocity[i] - KAPPA * height[i]) * dt
    for i in range(len(height)):
        height[i] += velocity[i] * dt


clock = 0.0
rays = {r: [] for r in range(2, 70)}
for frame in range(1, 91):
    elapsed = 0.1
    steps = math.ceil(elapsed * C / 0.5)
    for k in range(steps):
        dt = elapsed / steps
        now = clock + dt * (k + 1)
        drive(LOUDNESS * math.sin(OMEGA * now) * dt)
        step(dt)
        if now >= 7.7:
            for r in rays:
                rays[r].append(height[CENTRE[1] * COLUMNS + CENTRE[0] + r])
    clock += elapsed
envelope = {r: max(abs(v) for v in rays[r]) for r in rays}
alpha = GAMMA0 * OMEGA / (2 * C * math.sqrt(OMEGA * OMEGA - KAPPA))

# heat map: diverging, dark at zero, blue for troughs and orange for crests
scale = 1.6
pixels = []
for y in range(ROWS):
    for x in range(COLUMNS):
        t = max(-1.0, min(1.0, height[y * COLUMNS + x] / scale))
        base = (14, 17, 22)
        target = (217, 89, 38) if t > 0 else (57, 135, 229)
        w = abs(t) ** 0.8
        pixels.append(tuple(int(round(b + (g - b) * w)) for b, g in zip(base, target)))

figure = Figure(
    1180, 490,
    "One pebble on the pond",
    "Left: the height field of a single source at 0.8 hertz after nine seconds, with the border "
    "sponge visible as the fading of the rings at the edge. Right: the amplitude envelope along a "
    "ray against the Hankel-wave prediction, a power of one half in radius times a decaying exponential.",
)
figure.heading("A single 100% CPU pebble at 0.8 Hz (exact scheme, 151 by 85 cells)",
               "Crests orange, troughs blue. Wavelength 17.7 cells; the border sponge is the outer 10 cells.")
figure.png(40, 70, 604, 340, pixels, COLUMNS, ROWS)
figure.rect(40, 70, 604, 340, "none", AXIS, 1)
# sponge outline
figure.rect(40 + 40, 70 + 40, 604 - 80, 340 - 80, "none", MUTED, 1, None)
figure.text(40 + 46, 70 + 36, "sponge inner edge", 11, "start", MUTED)
figure.text(342, 430, "x (cells)", 12, "middle", INK)

radii = sorted(envelope)
ax = Axes(figure, 720, 90, 420, 320, (0, 70), (0.05, 5), ylog=True, xlabel="distance from the pebble (cells)",
          ylabel="amplitude envelope (height units)", title="Envelope along +x",
          xticks=[0, 10, 20, 30, 40, 50, 60, 70], yticks=[0.1, 1.0])
ax.vband(0, 3, MUTED, 0.1)
ax.scatter(radii, [envelope[r] for r in radii], BLUE, 2.2)
anchor = envelope[10] * math.sqrt(10) * math.exp(alpha * 10)
curve_r = [2 + 0.5 * i for i in range(137)]
ax.plot(curve_r, [anchor * r ** -0.5 * math.exp(-alpha * r) for r in curve_r], ORANGE, 2.0)
ax.plot(curve_r, [anchor * r ** -0.5 for r in curve_r], MUTED, 1.2, "4 3")
ax.legend([("measured (scheme)", BLUE, None),
           ("r^-1/2 exp(-0.0126 r)", ORANGE, None),
           ("r^-1/2 only", MUTED, "4 3")], "tr")
figure.text(720, 474, "The bump near 45 to 60 cells is the border reflection returning. Damping alone would give an e-folding length of 79 cells; geometric spreading adds the r^-1/2.", 11, "start", MUTED)
figure.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures", "ripple-single-source.svg"))
