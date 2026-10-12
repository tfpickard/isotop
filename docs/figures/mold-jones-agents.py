"""A Jones (2010) Physarum agent model on a trail grid, at the parameters isotop's mold view
is specified to use: SA 22.5 degrees, RA 45 degrees, SO 9 cells, SS 1 cell, deposit 5,
decay 0.1, 3 x 3 mean diffusion, one agent per cell.

Writes docs/media/figures/mold-jones-agents.svg. Run from the repository root:

    python3 docs/figures/mold-jones-agents.py

The grid is 128 x 128 here, not the 384 x 384 the view is specified to use, so that a pure
Python run takes under a minute; sensing range SO = 9 cells is therefore relatively three times
longer, and the picture shows the mechanism, not the view's scale. Food flakes emit attractant
in proportion to their CPU (halo exp(-2.5 (d - 1)) (0.15 + 0.012 cpu), as in the view's
specification) and one weak corridor joins two of them.
"""

import math
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

N = 128
SENSOR_ANGLE = math.radians(22.5)
ROTATE_ANGLE = math.radians(45.0)
SENSOR_OFFSET = 9.0
STEP_SIZE = 1.0
DEPOSIT = 5.0
DECAY = 0.1
AGENTS = 4200
STEPS = 500
SNAPSHOTS = (25, 120, 500)

# name, x, y, radius in cells, CPU percent
FLAKES = [("A", 30, 36, 5.0, 95.0), ("B", 98, 30, 4.0, 40.0), ("C", 62, 66, 3.5, 15.0),
          ("D", 28, 100, 4.5, 60.0), ("E", 100, 98, 3.5, 25.0), ("F", 62, 108, 2.2, 2.0)]
CORRIDOR = ("C", "D")


def food_field():
    food = [0.0] * (N * N)
    for _, fx, fy, radius, cpu in FLAKES:
        reach = int(radius + 14)
        for y in range(max(0, int(fy) - reach), min(N, int(fy) + reach)):
            for x in range(max(0, int(fx) - reach), min(N, int(fx) + reach)):
                d = math.hypot(x - fx, y - fy) / radius
                halo = math.exp(-2.5 * max(d - 1.0, 0.0)) * (0.15 + 0.012 * cpu)
                food[y * N + x] += halo
    return food


def corridor_field():
    by_name = {f[0]: f for f in FLAKES}
    a, b = by_name[CORRIDOR[0]], by_name[CORRIDOR[1]]
    field = [0.0] * (N * N)
    length = int(math.hypot(b[1] - a[1], b[2] - a[2]) * 2)
    for k in range(length + 1):
        t = k / length
        cx, cy = a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t
        for y in range(int(cy) - 3, int(cy) + 4):
            for x in range(int(cx) - 3, int(cx) + 4):
                if 0 <= x < N and 0 <= y < N:
                    d2 = (x - cx) ** 2 + (y - cy) ** 2
                    field[y * N + x] = max(field[y * N + x], 0.55 * math.exp(-d2 / (2 * 1.2**2)))
    return field


def blur(trail):
    """Separable 3 x 3 mean with zero padding outside the dish."""
    rows = []
    for y in range(N):
        row = trail[y * N:(y + 1) * N]
        left = [0.0] + row[:-1]
        right = row[1:] + [0.0]
        rows.append([(a + b + c) / 3.0 for a, b, c in zip(left, row, right)])
    out = [0.0] * (N * N)
    for y in range(N):
        up = rows[y - 1] if y > 0 else [0.0] * N
        down = rows[y + 1] if y + 1 < N else [0.0] * N
        mid = rows[y]
        out[y * N:(y + 1) * N] = [(a + b + c) / 3.0 for a, b, c in zip(up, mid, down)]
    return out


def simulate():
    rng = random.Random(2010)
    food = food_field()
    corridor = corridor_field()
    inside = [((x - 63.5) ** 2 + (y - 63.5) ** 2 < 61.0**2) for y in range(N) for x in range(N)]
    trail = [0.0] * (N * N)
    weights = [f[4] + 0.5 for f in FLAKES]
    ax, ay, ah = [], [], []
    for _ in range(AGENTS):
        flake = rng.choices(FLAKES, weights)[0]
        angle = rng.uniform(0, 2 * math.pi)
        radius = flake[3] * rng.uniform(1.1, 1.6)
        ax.append(flake[1] + radius * math.cos(angle))
        ay.append(flake[2] + radius * math.sin(angle))
        ah.append(angle + rng.gauss(0, 0.4))
    cos, sin = math.cos, math.sin
    shots = {}
    for step in range(1, STEPS + 1):
        for k in range(N * N):
            trail[k] += food[k] + 0.25 * corridor[k]
        occupied = bytearray(N * N)
        for i in range(AGENTS):
            occupied[int(ay[i]) * N + int(ax[i])] = 1
        deposits = []
        for i in range(AGENTS):
            x, y, h = ax[i], ay[i], ah[i]
            readings = []
            for turn in (0.0, SENSOR_ANGLE, -SENSOR_ANGLE):
                sx = int(x + SENSOR_OFFSET * cos(h + turn))
                sy = int(y + SENSOR_OFFSET * sin(h + turn))
                sx = 0 if sx < 0 else (N - 1 if sx >= N else sx)
                sy = 0 if sy < 0 else (N - 1 if sy >= N else sy)
                readings.append(trail[sy * N + sx])
            forward, left, right = readings
            if forward > left and forward > right:
                pass
            elif forward < left and forward < right:
                h += ROTATE_ANGLE if rng.random() < 0.5 else -ROTATE_ANGLE
            elif left < right:
                h -= ROTATE_ANGLE
            elif right < left:
                h += ROTATE_ANGLE
            nx, ny = x + STEP_SIZE * cos(h), y + STEP_SIZE * sin(h)
            cell = int(ny) * N + int(nx) if 0 <= nx < N and 0 <= ny < N else -1
            here = int(y) * N + int(x)
            if cell >= 0 and inside[cell] and (cell == here or not occupied[cell]):
                occupied[here] = 0 if cell != here else occupied[here]
                occupied[cell] = 1
                ax[i], ay[i], ah[i] = nx, ny, h
                deposits.append(cell)
            else:
                ah[i] = rng.uniform(0, 2 * math.pi)
        for cell in deposits:
            trail[cell] += DEPOSIT
        trail = blur(trail)
        decay = 1.0 - DECAY
        trail = [value * decay if inside[k] else 0.0 for k, value in enumerate(trail)]
        if step in SNAPSHOTS:
            shots[step] = trail[:]
    return shots


LEVELS = [(3.0, "#4a3a08"), (8.0, "#7a5b0a"), (16.0, "#b88a0e"), (30.0, "#e6b422"),
          (60.0, "#f6cf3c"), (110.0, "#fff0a0")]


def level_of(value):
    index = -1
    for k, (threshold, _) in enumerate(LEVELS):
        if value >= threshold:
            index = k
    return index


def draw_panel(svg, left, top, size, trail, title):
    scale = size / N
    svg.rect(left, top, size, size, fill="#10120a", stroke=P.AXIS, width=1.0)
    svg.circle(left + size / 2, top + size / 2, 61.0 * scale, fill="#14160c", stroke="#2a2d1e",
               width=1.0)
    svg.text(left, top - 9, title, size=12.5, weight="600")
    for y in range(N):
        x = 0
        while x < N:
            level = level_of(trail[y * N + x])
            if level < 0:
                x += 1
                continue
            start = x
            while x < N and level_of(trail[y * N + x]) == level:
                x += 1
            svg.rect(left + start * scale, top + y * scale, (x - start) * scale + 0.3,
                     scale + 0.3, fill=LEVELS[level][1])
    for name, fx, fy, radius, cpu in FLAKES:
        svg.circle(left + fx * scale, top + fy * scale, radius * scale, fill="#d9cfae",
                   stroke="#0e1116", width=1.2)
        svg.text(left + fx * scale, top + (fy - radius) * scale - 4, "%s %d%%" % (name, cpu),
                 size=10, anchor="middle", fill=P.TEXT)


def main():
    shots = simulate()
    size, gap, left, top = 330, 24, 24, 90
    svg = P.Svg(24 + 3 * (size + gap), top + size + 60,
                "Jones (2010) Physarum agents on a trail grid",
                "Trail concentration after 25, 120 and 500 steps with six food flakes of "
                "different CPU weights and a weak corridor between C and D.")
    svg.text(24, 28, "Jones agent model, %d agents, 128 x 128 grid" % AGENTS, size=16,
             weight="600")
    svg.text(24, 47, "SA 22.5, RA 45, SO 9, SS 1, deposit 5, decay 0.1, 3 x 3 mean; flake label "
             "= CPU percent; the corridor C-D is laid at 0.25 x 0.55 peak.", size=11,
             fill=P.MUTED)
    for index, step in enumerate(SNAPSHOTS):
        draw_panel(svg, left + index * (size + gap), top, size, shots[step], "step %d" % step)
    # Colour key.
    key_y = top + size + 26
    svg.text(24, key_y + 10, "trail", size=11, fill=P.MUTED)
    for index, (threshold, colour) in enumerate(LEVELS):
        svg.rect(70 + index * 62, key_y, 14, 14, fill=colour)
        svg.text(88 + index * 62, key_y + 11, ">=%g" % threshold, size=10.5, fill=P.MUTED)
    svg.save("docs/media/figures/mold-jones-agents.svg")


if __name__ == "__main__":
    main()
