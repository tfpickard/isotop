"""Reference simulations for the flocking figures (flocking-vicsek-order.py and
flocking-snapshots.py): the 1995 Vicsek rule on a periodic square, and the rule of the Coop yard
(src/coop/sim.rs, Yard::step) for one flock. Standard library only; every random stream is seeded.
"""

import math
import random

TAU = 2 * math.pi


def vicsek_periodic(n, side, eta, steps, burn, speed=0.03, radius=1.0, seed=1, state=False):
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
    if state:
        return total / count, list(zip(x, y)), theta
    return total / count


def yard(eta, steps=1200, average=300, n=60, seed=1, speed=0.6, h=0.05, radius=4.0, state=False):
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
    if state:
        return total / count, [tuple(p) for p in pos], theta
    return total / count


def noise(psi, cv=0.0):
    """eta of src/coop/sim.rs: ETA_BASE + K_CV min(cv, 2) + K_PSI psi / (psi + PSI_KNEE), at most 2 pi."""
    return min(0.5 + 0.5 * min(cv, 2.0) + 5.5 * psi / (psi + 15.0), TAU)
