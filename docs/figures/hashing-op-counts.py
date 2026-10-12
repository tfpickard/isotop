"""Writes docs/media/figures/hashing-op-counts.svg: operation counts (not wall time) of a uniform grid
against brute force: radius queries on uniform and clustered 2-D points, and 22-nearest queries on a
3-D flock, as a function of the cell width. The counts are those of simulation::SpatialHash and
SpatialHash3 (the shell search with its per-cell pruning), reproduced here in float64 from the same
splitmix64 stream; on the same points they equal the counts of an instrumented copy of the Rust.

Run from the repository root: python3 docs/figures/hashing-op-counts.py   (takes about half a minute)
"""
import math
import os
import sys
from collections import defaultdict

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w3 import *  # noqa: E402,F403

M64 = (1 << 64) - 1


class Rng:
    """simulation::Rng: splitmix64, unit() = top 24 bits, normal() = Box-Muller (cosine branch)."""

    def __init__(self, seed):
        self.state = seed & M64

    def next_u64(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & M64
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & M64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & M64
        return z ^ (z >> 31)

    def unit(self):
        return (self.next_u64() >> 40) / 16777216.0

    def signed(self):
        return self.unit() * 2.0 - 1.0

    def normal(self):
        radius = math.sqrt(-2.0 * math.log(1.0 - self.unit()))
        return radius * math.cos(math.tau * self.unit())


def uniform2(n, extent, seed):
    r = Rng(seed)
    return [(r.signed() * extent, r.signed() * extent) for _ in range(n)]


def clustered2(n, seed, blobs=6, sigma=4.0, spread=40.0):
    r = Rng(seed)
    centres = [(r.signed() * spread, r.signed() * spread) for _ in range(blobs)]
    out = []
    for i in range(n):
        c = centres[i % blobs]
        out.append((c[0] + r.normal() * sigma, c[1] + r.normal() * sigma))
    return out


def flock(count):
    threads = [1100, 400, 260, 180, 60, 220, 40, 20, 480, 90, 900]
    total = sum(threads)
    r = Rng(51)
    centres = [[r.normal() * 18.0, r.normal() * 18.0, r.normal() * 18.0] for _ in threads]
    pts = []
    for p, t in enumerate(threads):
        birds = count - len(pts) if p + 1 == len(threads) else count * t // total
        for _ in range(birds):
            c = centres[p]
            pts.append((c[0] + r.normal() * 14.0, c[1] + r.normal() * 14.0, c[2] + r.normal() * 14.0 + 60.0))
    return pts


def radius_counts(points, s, radius):
    n = len(points)
    grid = defaultdict(list)
    for i, p in enumerate(points):
        grid[(math.floor(p[0] / s), math.floor(p[1] / s))].append(i)
    lookups = tests = 0
    lim = radius * radius
    for p in points:
        lx, hx = math.floor((p[0] - radius) / s), math.floor((p[0] + radius) / s)
        ly, hy = math.floor((p[1] - radius) / s), math.floor((p[1] + radius) / s)
        if (hx - lx + 1) * (hy - ly + 1) > n:
            tests += n
            continue
        for cx in range(lx, hx + 1):
            for cy in range(ly, hy + 1):
                lookups += 1
                tests += len(grid.get((cx, cy), ()))
    return lookups / n, tests / n


def knn_counts(points, s, k):
    n = len(points)
    grid = defaultdict(list)
    for i, p in enumerate(points):
        grid[(math.floor(p[0] / s), math.floor(p[1] / s), math.floor(p[2] / s))].append(i)
    total_l = total_d = 0
    for p in points:
        q = tuple(math.floor(v / s) for v in p)
        inside = [p[a] - q[a] * s for a in range(3)]
        best, kth, lookups, dists, R = [], math.inf, 0, 0, 0

        def gap2(axis, off):
            g = 0.0 if off == 0 else (off * s - inside[axis] if off > 0 else inside[axis] + (-off - 1) * s)
            g = max(g, 0.0)
            return g * g

        done = False
        while (2 * R + 1) ** 3 <= n:
            limit = (kth * (1 + 1e-6)) ** 2 if kth < math.inf else math.inf
            kth2 = limit * (1 + 1e-6)
            for dx in range(-R, R + 1):
                gx = gap2(0, dx)
                if gx > limit:
                    continue
                for dy in range(-R, R + 1):
                    gxy = gx + gap2(1, dy)
                    if gxy > limit:
                        continue
                    zs = range(-R, R + 1) if (abs(dx) == R or abs(dy) == R) else ((-R, R) if R else (0,))
                    for dz in zs:
                        if gxy + gap2(2, dz) > limit:
                            continue
                        lookups += 1
                        for j in grid.get((q[0] + dx, q[1] + dy, q[2] + dz), ()):
                            o = points[j]
                            d2 = (o[0] - p[0]) ** 2 + (o[1] - p[1]) ** 2 + (o[2] - p[2]) ** 2
                            dists += 1
                            if d2 <= kth2:
                                best.append(math.sqrt(d2))
            if len(best) >= k:
                best.sort()
                del best[k:]
                kth = best[k - 1]
                face = min(min(p[a] - (q[a] - R) * s, (q[a] + R + 1) * s - p[a]) for a in range(3))
                if kth * (1 + 1e-6) <= face:
                    done = True
                    break
            R += 1
        if not done:
            dists += n
        total_l += lookups
        total_d += dists
    return total_l / n, total_d / n


def panel(svg, x, title, xr, yr, xs, tests, lookups, brute, extra=None, note=None):
    ax = Axes(svg, x, 60, 340, 300, xr, yr, True, True)
    ax.frame([0.3, 1, 3, 10, 30], [1, 10, 100, 1000, 10000], "cell width s", None, lambda v: "%g" % v, lambda v: "%g" % v)
    svg.text(x + 170, 36, title, 13, INK, "middle", weight="600")
    svg.line(ax.x, ax.py(brute), ax.x + ax.w, ax.py(brute), MUTED, 1.4, "6 4")
    svg.text(ax.x + 6, ax.py(brute) - 6, "brute force: %g tests" % brute, 11, MUTED)
    if extra:
        ax.path(xs, extra, YELLOW, 1.2, "2 3")
    ax.path(xs, tests, BLUE, 2.2)
    ax.path(xs, lookups, ORANGE, 2.0, "7 4")
    for a, b in zip(xs, tests):
        svg.circle(ax.px(a), ax.py(b), 3, BLUE)
    for a, b in zip(xs, lookups):
        svg.circle(ax.px(a), ax.py(b), 3, ORANGE)
    sums = [a + b for a, b in zip(tests, lookups)]
    best = sums.index(min(sums))
    svg.line(ax.px(xs[best]), ax.y, ax.px(xs[best]), ax.y + ax.h, AQUA, 1.2, "3 3")
    svg.text(ax.px(xs[best]) + 5, ax.y + ax.h - 8, "min of tests + lookups: s = %g" % xs[best], 11, AQUA)
    if note:
        svg.text(ax.x, ax.y + ax.h + 52, note, 11, MUTED)
    return ax


def main():
    svg = Svg(1200, 470, "Spatial hash: operation counts against brute force",
              "Distance tests and cell lookups per query against cell width for radius queries of 3 units on 4000 uniform "
              "points and on 4000 clustered points, and for 22-nearest queries on an 8000-point 3-D flock.")
    uni = uniform2(4000, 50.0, 7)
    clu = clustered2(4000, 9)
    r = 3.0
    cells = [0.3, 0.75, 1.5, 3.0, 6.0, 12.0, 24.0]
    ul = [radius_counts(uni, s, r) for s in cells]
    cl = [radius_counts(clu, s, r) for s in cells]
    rho = 4000 / 100.0 ** 2
    model = [rho * (2 * r + s) ** 2 for s in cells]
    panel(svg, 60, "radius 3, 4000 uniform points", (0.2, 40), (1, 20000), cells, [t for _, t in ul], [l for l, _ in ul], 4000, model,
          "dotted: model ρ (2r + s)² for the tests; lookups (2r/s + 1)² sit on the data")
    panel(svg, 450, "radius 3, 4000 points in 6 Gaussian clumps", (0.2, 40), (1, 20000), cells, [t for _, t in cl], [l for l, _ in cl], 4000, None,
          "111 hits per query (12 uniform): the optimum moves to finer cells")
    fcells = [2.5, 4, 5, 6.5, 8, 10, 13, 16, 24]
    pts = flock(8000)
    kn = [knn_counts(pts, s, 22) for s in fcells]
    panel(svg, 840, "22 nearest of an 8000-point 3-D flock", (2, 40), (1, 20000), fcells, [d for _, d in kn], [l for l, _ in kn], 8000, None,
          "cell 6.5 is the unit test's choice: 190 tests, 41 lookups")
    svg.line(60, 438, 90, 438, BLUE, 2.2)
    svg.text(96, 442, "distance tests per query", 11, INK)
    svg.line(280, 438, 310, 438, ORANGE, 2, "7 4")
    svg.text(316, 442, "cell lookups per query", 11, INK)
    svg.text(500, 442, "counts of SpatialHash / SpatialHash3 on seeded points; 3· per point, not wall time", 11, MUTED)
    svg.save("docs/media/figures/hashing-op-counts.svg")


main()
