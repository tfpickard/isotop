"""Droplet hydraulic erosion (Beyer 2015) with the canyon brief's constants, in pure Python.

Run from the repository root: python3 docs/figures/canyon-erosion.py   (about a minute)
Writes docs/media/figures/canyon-erosion.svg.

A 96 x 96 heightfield: a gentle slope draining to the far corner with one Gaussian mesa (a big,
quiet process: sigma = 11 cells, height 0.42) and nothing else. Rain: 1500 droplets released around
the mesa plot (amber, a session process) and 4500 around a second plot on the slope (violet, a busy
container); one droplet stands for 10 ms of CPU time. Constants from docs/mockups/code/canyon.py
and issue #55: inertia 0.05, capacity factor 4, minimum capacity 0.0008, erode 0.15, deposit 0.3,
evaporation 0.01, gravity 4, lifetime 70 steps, brush radius 3. Every cell stores a sediment weight
and an RGB sum, so deposited sediment keeps the colour of the rain that carried it; erosion removes
both in proportion. No hardness, no uplift, no lakes. Panels: shaded relief, height change, sediment.
"""

import math

import svgkit_w8 as kit
from izh_w8 import Normal

N = 96
INERTIA, CAPACITY, MIN_CAPACITY = 0.05, 4.0, 0.0008
ERODE, DEPOSIT, EVAPORATE, GRAVITY, LIFE, RADIUS = 0.15, 0.3, 0.01, 4.0, 70, 3
AMBER, VIOLET = (242.0, 176.0, 64.0), (172.0, 128.0, 242.0)


def make_terrain():
    h = [[0.0] * N for _ in range(N)]
    for y in range(N):
        for x in range(N):
            slope = 0.10 + 0.14 * (1.0 - (x + y) / (2.0 * (N - 1)))
            mesa = 0.42 * math.exp(-((x - 34) ** 2 + (y - 32) ** 2) / (2 * 11.0 ** 2))
            h[y][x] = slope + mesa
    return h


BRUSH = []
for dy in range(-RADIUS, RADIUS + 1):
    for dx in range(-RADIUS, RADIUS + 1):
        d = math.hypot(dx, dy)
        if d < RADIUS:
            BRUSH.append((dx, dy, RADIUS - d))
TOTAL = sum(w for _, _, w in BRUSH)
BRUSH = [(dx, dy, w / TOTAL) for dx, dy, w in BRUSH]


def sample(h, x, y):
    ix, iy = int(x), int(y)
    fx, fy = x - ix, y - iy
    h00, h10, h01, h11 = h[iy][ix], h[iy][ix + 1], h[iy + 1][ix], h[iy + 1][ix + 1]
    gx = (h10 - h00) * (1 - fy) + (h11 - h01) * fy
    gy = (h01 - h00) * (1 - fx) + (h11 - h10) * fx
    height = h00 * (1 - fx) * (1 - fy) + h10 * fx * (1 - fy) + h01 * (1 - fx) * fy + h11 * fx * fy
    return height, gx, gy


def droplet(h, weight, colour, x, y):
    dx = dy = 0.0
    speed, water, sediment = 1.0, 1.0, 0.0
    for _ in range(LIFE):
        ix, iy = int(x), int(y)
        fx, fy = x - ix, y - iy
        height, gx, gy = sample(h, x, y)
        dx = dx * INERTIA - gx * (1 - INERTIA)
        dy = dy * INERTIA - gy * (1 - INERTIA)
        length = math.hypot(dx, dy)
        if length < 1e-9:
            break
        dx, dy = dx / length, dy / length
        nx, ny = x + dx, y + dy
        if not (0 <= nx < N - 1.01 and 0 <= ny < N - 1.01):
            break
        dh = sample(h, nx, ny)[0] - height
        capacity = max(-dh * speed * water * CAPACITY, MIN_CAPACITY)
        if sediment > capacity or dh > 0:
            amount = min(dh, sediment) if dh > 0 else (sediment - capacity) * DEPOSIT
            sediment -= amount
            parts = ((ix, iy, (1 - fx) * (1 - fy)), (ix + 1, iy, fx * (1 - fy)),
                     (ix, iy + 1, (1 - fx) * fy), (ix + 1, iy + 1, fx * fy))
            for cx, cy, w in parts:
                give = amount * w
                h[cy][cx] += give
                weight[cy][cx] += give
                for a in range(3):
                    colour[cy][cx][a] += give * carried[a]
        else:
            amount = min((capacity - sediment) * ERODE, -dh)
            for bx, by, w in BRUSH:
                cx, cy = ix + bx, iy + by
                if 0 <= cx < N and 0 <= cy < N:
                    take = min(amount * w, h[cy][cx])
                    h[cy][cx] -= take
                    if weight[cy][cx] > 1e-12:
                        frac = min(take / weight[cy][cx], 1.0)
                        lost = weight[cy][cx] * frac
                        weight[cy][cx] -= lost
                        for a in range(3):
                            colour[cy][cx][a] *= (1 - frac)
            sediment += amount
        speed = math.sqrt(max(speed * speed + dh * GRAVITY, 0.0))
        water *= 1 - EVAPORATE
        x, y = nx, ny


carried = list(AMBER)


def main():
    global carried
    h = make_terrain()
    before = [row[:] for row in h]
    weight = [[0.0] * N for _ in range(N)]
    colour = [[[0.0, 0.0, 0.0] for _ in range(N)] for _ in range(N)]
    rng = Normal(21)
    plots = [((34.0, 32.0), 1500, AMBER), ((64.0, 58.0), 4500, VIOLET)]
    released = 0
    for (px, py), count, rgb in plots:
        carried = list(rgb)
        for _ in range(count):
            x = min(max(px + 9.0 * rng.normal(), 2.0), N - 3.0)
            y = min(max(py + 9.0 * rng.normal(), 2.0), N - 3.0)
            droplet(h, weight, colour, x, y)
            released += 1
    # relief shading
    def shade(field):
        rows = []
        for y in range(N):
            row = []
            for x in range(N):
                gx = (field[y][min(x + 1, N - 1)] - field[y][max(x - 1, 0)]) * 0.5
                gy = (field[min(y + 1, N - 1)][x] - field[max(y - 1, 0)][x]) * 0.5
                lit = 0.6 - 14.0 * (0.7 * gx + 0.7 * gy)
                lit = min(max(lit, 0.15), 1.1)
                base = 0.7 + 0.3 * min(max((field[y][x] - 0.1) / 0.5, 0.0), 1.0)
                row.append((228 * lit * base, 176 * lit * base, 130 * lit * base))
            rows.append(row)
        return rows
    svg = kit.Svg(960, 400, "Droplet erosion with the canyon constants",
                  "Shaded relief, height change and sediment colour after 6000 droplets on a 96 by "
                  "96 heightfield with one mesa.")
    svg.title_text("canyon (planned): %d droplets on a 96 x 96 field, a mesa and two rain plots" % released)
    scale, top = 3.0, 70
    size = N * scale
    xs = [30, 340, 650]
    svg.image(xs[0], top, size, size, kit.rgb_png(shade(h)))
    diff = [[h[y][x] - before[y][x] for x in range(N)] for y in range(N)]
    pixels = []
    for y in range(N):
        row = []
        for x in range(N):
            d = diff[y][x]
            t = min(abs(d) / 0.03, 1.0)
            row.append((25 + 215 * t, 28 + 90 * t, 40 + 20 * t) if d > 0 else (25 + 30 * t, 28 + 110 * t, 40 + 205 * t))
        pixels.append(row)
    svg.image(xs[1], top, size, size, kit.rgb_png(pixels))
    pixels = []
    for y in range(N):
        row = []
        for x in range(N):
            w = weight[y][x]
            if w > 1e-5:
                mix = [colour[y][x][a] / w for a in range(3)]
                t = min(w / 0.002, 1.0)
                row.append(tuple(18 + (mix[a] - 18) * t for a in range(3)))
            else:
                row.append((18, 20, 28))
        pixels.append(row)
    svg.image(xs[2], top, size, size, kit.rgb_png(pixels))
    for x, caption in zip(xs, ("a  relief after rain", "b  height change (blue cut, orange fill)",
                              "c  sediment colour = the rain that carried it")):
        svg.text(x, top - 10, caption, size=12, weight="600")
        svg.rect(x, top, size, size, stroke=kit.AXIS)
    for (px, py), count, rgb in plots:
        for x in xs:
            svg.circle(x + px * scale, top + py * scale, 3, fill="none", stroke="#ffffff", width=1.2)
    svg.text(480, 392, "white rings: the two rain plots. Mesa plot: 1500 droplets; busy plot: 4500. "
             "Constants: inertia 0.05, capacity 4, erode 0.15, deposit 0.3, evaporation 0.01, life 70.",
             size=10.5, anchor="middle", fill=kit.MUTED)
    svg.save("canyon-erosion")
    cut = min(min(row) for row in diff)
    fill = max(max(row) for row in diff)
    removed = sum(-d for row in diff for d in row if d < 0)
    added = sum(d for row in diff for d in row if d > 0)
    print("deepest cut %.4f, highest fill %.4f, total removed %.2f added %.2f" % (cut, fill, removed, added))
    print("height range before %.3f..%.3f after %.3f..%.3f" % (
        min(min(r) for r in before), max(max(r) for r in before), min(min(r) for r in h), max(max(r) for r in h)))


if __name__ == "__main__":
    main()
