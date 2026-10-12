"""The order parameter of a Vicsek flock against its heading noise, computed twice.

Run from the repository root: python3 docs/figures/flocking-vicsek-order.py
Writes docs/media/figures/flocking-vicsek-order.svg. Standard library only; every random stream is
seeded, so the output is the same every run (it takes a couple of minutes).

Series A is the 1995 rule on a periodic square: N particles of constant speed v0 set their new
heading to the mean heading of everything within r (themselves included) plus a uniform kick in
[-eta/2, eta/2], then step. Series B re-implements the Coop yard's rule (src/coop.rs, Yard::step) for
one flock of 60 foragers: the same mean-heading rule with the same noise convention, a speed of
0.6 units/s at the 20 Hz step, an interaction radius of 4 units, the henhouse's pull beyond range 10,
its footprint, the reflecting fence and the Jacobi separation of overlapping bodies. It uses
Python's random module rather than the Rust generator, so it follows the rule, not the bits.
The mapping from CPU pressure psi to eta is the one in coop.rs: eta = 0.5 + 0.5 min(CV, 2) +
5.5 psi / (psi + 15), capped at 2 pi.
"""

import math
import os
import random
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import (AQUA, BLUE, MAGENTA, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg,
                       heading)

TAU = 2 * math.pi


def vicsek_periodic(n, side, eta, steps, burn, speed=0.03, radius=1.0, seed=1):
    """Mean order parameter over the last steps - burn steps of the 1995 model."""
    rng = random.Random(seed)
    x = [rng.random() * side for _ in range(n)]
    y = [rng.random() * side for _ in range(n)]
    theta = [rng.random() * TAU for _ in range(n)]
    cells = int(side // radius)
    size = side / cells
    total, count = 0.0, 0
    for step in range(steps):
        grid = {}
        for i in range(n):
            grid.setdefault((int(x[i] / size) % cells, int(y[i] / size) % cells), []).append(i)
        c = [math.cos(a) for a in theta]
        s = [math.sin(a) for a in theta]
        new = [0.0] * n
        for i in range(n):
            cx, cy = int(x[i] / size) % cells, int(y[i] / size) % cells
            sx = sy = 0.0
            for ox in (-1, 0, 1):
                for oy in (-1, 0, 1):
                    for j in grid.get(((cx + ox) % cells, (cy + oy) % cells), ()):
                        dx, dy = abs(x[i] - x[j]), abs(y[i] - y[j])
                        dx = side - dx if dx > side / 2 else dx
                        dy = side - dy if dy > side / 2 else dy
                        if dx * dx + dy * dy <= radius * radius:
                            sx += c[j]
                            sy += s[j]
            new[i] = math.atan2(sy, sx) + eta * (rng.random() - 0.5)
        theta = new
        for i in range(n):
            x[i] = (x[i] + speed * math.cos(theta[i])) % side
            y[i] = (y[i] + speed * math.sin(theta[i])) % side
        if step >= burn:
            total += math.hypot(sum(math.cos(a) for a in theta),
                                sum(math.sin(a) for a in theta)) / n
            count += 1
    return total / count


def yard(eta, steps=1200, average=300, n=60, seed=1, speed=0.6, h=0.05, radius=4.0):
    """The Coop yard rule for one flock; returns the mean order over the last `average` steps."""
    rng = random.Random(seed)
    pos = [[8.0 + (rng.random() * 2 - 1) * 9.0, 8.0 + (rng.random() * 2 - 1) * 9.0]
           for _ in range(n)]
    theta = [rng.random() * TAU for _ in range(n)]
    footprint, home_range, body = 4.0, 10.0, 0.3
    low, high = -30.0, 30.0
    r2 = radius * radius
    total, count = 0.0, 0
    for step in range(steps):
        draws = [(rng.random() * 2 - 1) * 0.5 for _ in range(n)]
        c = [math.cos(a) for a in theta]
        s = [math.sin(a) for a in theta]
        new = theta[:]
        for i in range(n):
            xi, yi = pos[i]
            sx = sy = 0.0
            for j in range(n):
                dx, dy = pos[j][0] - xi, pos[j][1] - yi
                if dx * dx + dy * dy <= r2:
                    sx += c[j]
                    sy += s[j]
            d = math.hypot(xi, yi)
            if d > home_range:
                pull = 0.6 * min((d - home_range) / 4.0, 1.0)
                sx += pull * -xi / d
                sy += pull * -yi / d
            base = math.atan2(sy, sx) if math.hypot(sx, sy) > 1e-6 else theta[i]
            new[i] = (base + eta * draws[i]) % TAU
        theta = new
        for i in range(n):
            pos[i][0] += speed * h * math.cos(theta[i])
            pos[i][1] += speed * h * math.sin(theta[i])
        pushes = [[0.0, 0.0] for _ in range(n)]
        for i in range(n):
            for j in range(n):
                if i == j:
                    continue
                dx, dy = pos[i][0] - pos[j][0], pos[i][1] - pos[j][1]
                dist = math.hypot(dx, dy)
                overlap = 2 * body - dist
                if overlap > 0:
                    ux, uy = (dx / dist, dy / dist) if dist > 1e-5 else (1.0, 0.0)
                    pushes[i][0] += ux * overlap * 0.5
                    pushes[i][1] += uy * overlap * 0.5
        for i in range(n):
            pos[i][0] += pushes[i][0]
            pos[i][1] += pushes[i][1]
        for i in range(n):
            ax, ay = pos[i][0], pos[i][1]
            d = math.hypot(ax, ay)
            limit = footprint + body
            if d < limit:
                nx, ny = (ax / d, ay / d) if d > 1e-5 else (math.cos(theta[i]), math.sin(theta[i]))
                pos[i][0], pos[i][1] = nx * limit, ny * limit
                inward = math.cos(theta[i]) * nx + math.sin(theta[i]) * ny
                if inward < 0:
                    hx = math.cos(theta[i]) - 2 * inward * nx
                    hy = math.sin(theta[i]) - 2 * inward * ny
                    theta[i] = math.atan2(hy, hx) % TAU
            for axis in (0, 1):
                lo, hi = low + body, high - body
                value = pos[i][axis]
                if value < lo or value > hi:
                    edge = lo if value < lo else hi
                    pos[i][axis] = 2 * edge - value
                    theta[i] = ((math.pi - theta[i]) if axis == 0 else -theta[i]) % TAU
        if step >= steps - average:
            total += math.hypot(sum(math.cos(a) for a in theta),
                                sum(math.sin(a) for a in theta)) / n
            count += 1
    return total / count


def noise(psi, cv=0.0):
    """eta of src/coop.rs: ETA_BASE + K_CV min(cv, 2) + K_PSI psi / (psi + PSI_KNEE), at most 2 pi."""
    return min(0.5 + 0.5 * min(cv, 2.0) + 5.5 * psi / (psi + 15.0), TAU)


def interpolate(xs, ys, x):
    if x <= xs[0]:
        return ys[0]
    for k in range(1, len(xs)):
        if x <= xs[k]:
            t = (x - xs[k - 1]) / (xs[k] - xs[k - 1])
            return ys[k - 1] + t * (ys[k] - ys[k - 1])
    return ys[-1]


def main():
    etas = [0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, TAU]
    periodic = [vicsek_periodic(256, 6.4, e, 1000, 500, seed=3) for e in etas]
    seeds = [11, 12, 13, 14, 15, 16]
    runs = [[yard(e, seed=s) for s in seeds] for e in etas]
    means = [sum(r) / len(r) for r in runs]

    svg = Svg(1080, 430, "Vicsek order parameter against heading noise",
              "Order parameter against heading noise for the 1995 Vicsek rule and for the Coop "
              "yard rule, and the noise a given CPU pressure produces in the coop.")
    heading(svg, "Order against noise: the 1995 rule and the Coop yard",
            "phi = |sum of velocities| / sum of speeds; each point is a time average after a burn-in (1000 steps, the last 500 for the periodic box; 1200 steps, the last 300 for the yard)")

    left = Axes(svg, 62, 82, 400, 270, (0, TAU), (0, 1.05), "heading noise eta (radians)",
                "order parameter phi", "A: phi(eta)",
                xticks=[0, 1, 2, 3, 4, 5, 6], yticks=[0, 0.2, 0.4, 0.6, 0.8, 1.0])
    left.hline(0.886 / math.sqrt(256), BLUE, 1, "2 4")
    left.hline(0.886 / math.sqrt(60), ORANGE, 1, "2 4")
    left.line(etas, periodic, BLUE, 2.2)
    left.points(etas, periodic, BLUE, 3)
    for k, eta in enumerate(etas):
        left.points([eta] * len(seeds), runs[k], ORANGE, 1.6, opacity=0.45)
    left.line(etas, means, ORANGE, 2.2)
    left.points(etas, means, ORANGE, 3)
    left.legend([("1995 rule, periodic box (N = 256, r = 1, v0 = 0.03)", BLUE, "line"),
                 ("Coop yard rule (N = 60, r = 4, 0.6 u/s, fence)", ORANGE, "line"),
                 ("dotted: random-heading floor 0.886 / sqrt(N)", MUTED, "dash")],
                x=left.x + 150, y=left.y + 26, size=10.5)
    left.vline(noise(0), VIOLET, 1, "3 3")
    left.label(noise(0), 0.07, "psi = 0", 10, "start", VIOLET, dx=4)
    left.vline(noise(40), VIOLET, 1, "3 3")
    left.label(noise(40), 0.07, "psi = 40", 10, "start", VIOLET, dx=4)

    mid = Axes(svg, 540, 82, 220, 270, (0, 100), (0, 2 * math.pi + 0.2),
               "CPU pressure psi (%)", "eta (radians)", "B: eta(psi)",
               xticks=[0, 20, 40, 60, 80, 100], yticks=[0, 1, 2, 3, 4, 5, 6])
    psis = [k * 0.5 for k in range(0, 201)]
    for cv, color in ((0.0, AQUA), (1.0, YELLOW), (2.0, MAGENTA)):
        mid.line(psis, [noise(p, cv) for p in psis], color, 2)
    mid.hline(TAU, MUTED, 1, "4 3")
    mid.label(100, TAU, "2 pi cap", 10, "end", MUTED, dy=-5)
    mid.legend([("CV = 0", AQUA, "line"), ("CV = 1", YELLOW, "line"), ("CV = 2", MAGENTA, "line")],
               x=mid.x + 56, y=mid.y + 188, size=10.5)

    right = Axes(svg, 836, 82, 220, 270, (0, 100), (0, 1.05),
                 "CPU pressure psi (%)", "order parameter phi", "C: phi(psi), Coop rule",
                 xticks=[0, 20, 40, 60, 80, 100], yticks=[0, 0.2, 0.4, 0.6, 0.8, 1.0])
    for cv, color in ((0.0, AQUA), (1.0, YELLOW), (2.0, MAGENTA)):
        right.line(psis, [interpolate(etas, means, noise(p, cv)) for p in psis], color, 2)
    right.hline(0.886 / math.sqrt(60), ORANGE, 1, "2 4")
    right.legend([("CV = 0", AQUA, "line"), ("CV = 1", YELLOW, "line"), ("CV = 2", MAGENTA, "line")],
                 x=right.x + 56, y=right.y + 20, size=10.5)
    svg.text(24, 418, "Panel C reads panel A through panel B: it interpolates the orange mean at "
                      "eta(psi, CV). Orange dots in A are the six seeds, each a time average over the last 15 s.", 10.5, "start", MUTED)
    print(svg.save("flocking-vicsek-order"))
    for e, p, m in zip(etas, periodic, means):
        print("eta %.2f  periodic %.3f  yard %.3f" % (e, p, m))


if __name__ == "__main__":
    main()
