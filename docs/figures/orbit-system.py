"""One Orbit system from the demo workload, and the period law that moves it.

Writes docs/media/figures/orbit-system.svg. Run from the repository root:
    python3 docs/figures/orbit-system.py
The layout is a stdlib port of mass_radius, ring_scale, elements, shells, eccentric_anomaly and
period in src/render.rs, fed with the demo workload at --time 12 (family 1: a head and fifteen
children). Left: plan view at t = 0 (filled) and 15 s later (outlined). Right: T(a) for several
parent subtree memories with the 8 s to 600 s clamp, and the system's own shells marked.
Stdlib only, deterministic.
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w4 import *  # noqa: E402,F401,F403

TAU = 2 * math.pi
ORBIT_GAP = 0.6
MIB = 1048576.0


def knuth(value):
    return (value * 2654435761) & 0xFFFFFFFF


def elements(pid):
    h = knuth(pid)
    return 0.06 + 0.16 * (h >> 24) / 255.0, TAU * ((h >> 8) & 0xFFFF) / 65536.0


def mass_radius(bytes_):
    return min(max(0.24 * max(bytes_ / MIB, 0.0) ** (1 / 3), 0.25), 4.5)


def ring_scale(threads):
    count = 0 if threads <= 3 else 1 if threads <= 15 else 2 if threads <= 63 else 3 if threads <= 255 else 4
    return 1.0 if count == 0 else 1.55 + 0.28 * (count - 1)


def period(a, parent_bytes):
    mass = max(parent_bytes / (512 * MIB), 1 / 512)
    return min(max(30.0 * (a / 3.0) ** 1.5 / math.sqrt(mass), 8.0), 600.0)


def raw_period(a, parent_bytes):
    mass = max(parent_bytes / (512 * MIB), 1 / 512)
    return 30.0 * (a / 3.0) ** 1.5 / math.sqrt(mass)


def eccentric(mean, e):
    mean %= TAU
    anomaly = mean
    for _ in range(6):
        anomaly -= (anomaly - e * math.sin(anomaly) - mean) / (1 - e * math.cos(anomaly))
    return anomaly


def shells(body, sizes, e):
    squeeze = math.sqrt((1 - e) / (1 + e))
    n = len(sizes)
    widest_after = [0.0] * (n + 1)
    for i in range(n - 1, -1, -1):
        widest_after[i] = max(widest_after[i + 1], sizes[i])
    slots = []
    a = (body + widest_after[0] + ORBIT_GAP + 1.0) / (1 - e)
    start = shell = 0
    while start < n:
        capacity = TAU * a * squeeze
        end, used = start, 0.0
        while end < n and (end == start or used + 2 * sizes[end] + ORBIT_GAP <= capacity):
            used += 2 * sizes[end] + ORBIT_GAP
            end += 1
        slack = max(capacity - used, 0.0) / (end - start)
        mean, widest = shell * 0.5, 0.0
        for size in sizes[start:end]:
            share = (2 * size + ORBIT_GAP + slack) / (a * squeeze)
            slots.append((a, mean + share * 0.5))
            mean += share
            widest = max(widest, size)
        start = end
        if start < n:
            a += (widest + widest_after[start] + ORBIT_GAP) / (1 - e)
        shell += 1
    return slots


def demo(index, time=12.0):
    phase = time * 0.55 + index * 1.71
    memory = (12.0 + (index * 71 % 900) * (0.8 + 0.2 * math.sin(phase * 0.2))) * MIB
    threads = (4 << (index % 6)) if index % 5 == 0 else 1
    return memory, threads


FAMILY = 1
head = FAMILY * 16
kids = [head + j for j in range(1, 16)]
head_memory, head_threads = demo(head)
kid_memory = [demo(i)[0] for i in kids]
total = head_memory + sum(kid_memory)
e, omega = elements(1000 + head)
reach = [mass_radius(m) * ring_scale(demo(i)[1]) for m, i in zip(kid_memory, kids)]
slots = shells(mass_radius(total) * ring_scale(head_threads), reach, e)
axes_used = sorted(set(round(a, 4) for a, _ in slots))


def position(a, mean):
    anomaly = eccentric(mean, e)
    x, y = a * (math.cos(anomaly) - e), a * math.sqrt(1 - e * e) * math.sin(anomaly)
    return x * math.cos(omega) - y * math.sin(omega), x * math.sin(omega) + y * math.cos(omega)


figure = Figure(
    1220, 580,
    "An Orbit system and its period law",
    "Left: plan view of one demo system, a head process and its fifteen children on two confocal "
    "ellipses, at time zero and fifteen seconds later; inner bodies have moved further. Right: the "
    "orbital period against semi-major axis for several parent memories, clamped to 8 to 600 seconds.",
)
figure.heading("One system from the demo (family 1) and the third-law period",
               "Kepler motion about a focus; semi-major axes come from packing, periods from T = 30 (a/3)^1.5 / sqrt(M / 512 MiB).")

# left panel: plan view
size = 420
left, top = 40, 80
scale = size / 2 / (max(axes_used) * (1 + e) + 4)
cx, cy = left + size / 2, top + size / 2


def to_screen(point):
    return cx + point[0] * scale, cy - point[1] * scale


figure.rect(left, top, size, size, "none", AXIS, 1)
for a in axes_used:
    curve = [to_screen(position(a, TAU * i / 240)) for i in range(241)]
    figure.polyline(curve, MUTED, 1.0)
head_radius = mass_radius(total)
sx, sy = to_screen((0, 0))
figure.circle(sx, sy, head_radius * scale, YELLOW, BG, 1, 0.9)
figure.text(sx, sy + 4, "star", 10, "middle", BG, weight="600")
elapsed = 15.0
for (a, mean), r in zip(slots, reach):
    x0, y0 = to_screen(position(a, mean))
    figure.circle(x0, y0, max(r * scale * 0.55, 2.2), BLUE, BG, 1, 0.9)
    later = mean + TAU * elapsed / period(a, total)
    x1, y1 = to_screen(position(a, later))
    figure.circle(x1, y1, max(r * scale * 0.55, 2.2), "none", ORANGE, 1.4)
inner = axes_used[0]
figure.text(left + 8, top + size + 22, "filled: t = 0; outlined: t = 15 s.", 11, "start", MUTED)
figure.text(left + 8, top + size + 38, "periapsis directions share omega = %.2f rad, e = %.3f (hash of the head's pid)." % (omega, e), 11, "start", MUTED)
for index, a in enumerate(axes_used):
    figure.text(left + size - 8, top + 18 + 16 * index, "shell %d: a = %.1f, T = %.0f s" % (index + 1, a, period(a, total)), 11, "end", INK)
figure.text(left + size - 8, top + 18 + 16 * len(axes_used), "subtree memory %.1f GiB" % (total / 2**30), 11, "end", MUTED)

# right panel: T(a)
ax = Axes(figure, 600, 90, 570, 360, (1, 40), (3, 40000), xlog=True, ylog=True,
          xlabel="semi-major axis a (world units)", ylabel="period T (s)", title="Third law with the display clamp",
          xticks=[1, 2, 3, 5, 10, 20, 40], yticks=[10, 100, 1000, 10000], yminor=False, yformat=lambda v: "%g" % v)
ax.figure.rect(ax.x, ax.Y(600), ax.w, ax.Y(8) - ax.Y(600), AQUA, None, 1, 0.1)
ax.hline(600, MUTED, 1.0, "4 3", "clamp 600 s", 1.2)
ax.hline(8, MUTED, 1.0, "4 3", "clamp 8 s", 1.2)
grid = [10 ** (math.log10(1) + i / 120 * math.log10(40)) for i in range(121)]
for memory_mib, color, dash, name in ((1, VIOLET, "6 3", "no memory (floor 1 MiB)"), (64, MAGENTA, None, "64 MiB"),
                                      (512, YELLOW, None, "512 MiB"), (4096, AQUA, None, "4 GiB"),
                                      (total / MIB, BLUE, "2 2", "this system, %.1f GiB" % (total / 2**30))):
    ax.plot(grid, [raw_period(a, memory_mib * MIB) for a in grid], color, 1.4, dash, opacity=0.45)
    ax.plot(grid, [period(a, memory_mib * MIB) for a in grid], color, 2.2, dash)
ax.scatter(axes_used, [period(a, total) for a in axes_used], BLUE, 5)
ax.legend([("no memory (floor 1 MiB)", VIOLET, "6 3"), ("64 MiB", MAGENTA, None), ("512 MiB", YELLOW, None),
           ("4 GiB", AQUA, None), ("this system", BLUE, "2 2")], "br")
figure.text(600, 532, "Faint lines are the unclamped law; slope 3/2. Dots are this system's shells.", 11, "start", MUTED)
figure.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures", "orbit-system.svg"))
