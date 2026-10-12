"""The murmuration prototype's flock (docs/mockups/code/murmuration.py) in pure Python at a smaller N.

Constants are the brief's: alignment 0.30 over the 7 nearest (same-process neighbours count
double), cohesion 0.02 over the 21 nearest, separation 0.4 within 1.5, whole-flock pull 0.0006,
roost 0.004 beyond r = 80 and an altitude band 35 to 110 with 0.01, vertical damping 0.10, noise
0.02, speed 1 + 0.3 busy, falcon radius 14 and acceleration 0.9, agitation spread 0.93 above 0.35,
decay 0.86, jitter 0.12. Brute-force neighbours (heapq), so keep N in the hundreds."""

import heapq
import math

from izh_w8 import Normal

PROCESSES = [110, 40, 26, 18, 6, 22, 4, 2, 48, 9, 90]  # threads (the prototype's list / 10)
BUSY = [0.15, 0.9, 0.3, 0.2, 0.4, 0.2, 0.02, 0.05, 0.6, 0.1, 0.05]


class Flock:
    def __init__(self, seed=11):
        self.rng = Normal(seed)
        r = self.rng
        self.owner, self.speed = [], []
        for index, (threads, share) in enumerate(zip(PROCESSES, BUSY)):
            for _ in range(threads):
                self.owner.append(index)
                self.speed.append(1.0 + 0.3 * (1.0 if r.unit() < share else 0.0))
        n = self.n = len(self.owner)
        centres = [[18.0 * r.normal() for _ in range(3)] for _ in PROCESSES]
        self.pos = [[centres[o][a] + 14.0 * r.normal() + (60.0 if a == 2 else 0.0) for a in range(3)]
                    for o in self.owner]
        hx, hy = r.normal(), r.normal()
        hn = math.hypot(hx, hy)
        self.vel = [[hx / hn + 0.3 * r.normal(), hy / hn + 0.3 * r.normal(), 0.3 * r.normal()]
                    for _ in range(n)]
        self.agitation = [0.0] * n
        self.falcon = None
        self.neighbours = [[] for _ in range(n)]

    def knn(self, count=21):
        pos = self.pos
        out = []
        for i, p in enumerate(pos):
            px, py, pz = p
            nearest = heapq.nsmallest(
                count + 1,
                ((((q[0] - px) ** 2 + (q[1] - py) ** 2 + (q[2] - pz) ** 2), j)
                 for j, q in enumerate(pos)))
            out.append([(math.sqrt(d2), j) for d2, j in nearest if j != i][:count])
        self.neighbours = out
        return out

    def step(self):
        n, pos, vel, owner, rng = self.n, self.pos, self.vel, self.owner, self.rng
        nb = self.knn(21)
        mean = [sum(p[a] for p in pos) / n for a in range(3)]
        new_vel = []
        for i in range(n):
            near7 = nb[i][:7]
            weights = [(2.0 if owner[j] == owner[i] else 1.0) for _, j in near7]
            total = sum(weights)
            nv = [sum(w * vel[j][a] for w, (_, j) in zip(weights, near7)) / total for a in range(3)]
            npos = [sum(pos[j][a] for _, j in nb[i]) / len(nb[i]) for a in range(3)]
            sep = [0.0, 0.0, 0.0]
            for d, j in near7:
                if d < 1.5:
                    scale = 1.0 / max(d, 0.3) ** 2
                    for a in range(3):
                        sep[a] += (pos[i][a] - pos[j][a]) * scale
            p, v = pos[i], vel[i]
            radius = math.hypot(p[0], p[1])
            pull = max(radius - 80.0, 0.0) * 0.004
            roost = [-p[0] / max(radius, 1.0) * pull, -p[1] / max(radius, 1.0) * pull,
                     max(35.0 - p[2], 0.0) * 0.01 - max(p[2] - 110.0, 0.0) * 0.01]
            steer = [0.30 * (nv[a] - v[a]) + 0.02 * (npos[a] - p[a]) + 0.4 * sep[a]
                     + 0.0006 * (mean[a] - p[a]) + roost[a] + 0.02 * rng.normal() for a in range(3)]
            steer[2] -= 0.10 * v[2]
            if self.falcon is not None:
                fp, fv = self.falcon
                off = [p[a] - fp[a] for a in range(3)]
                dist = math.sqrt(sum(x * x for x in off))
                if dist < 14.0:
                    self.agitation[i] = 1.0
                    for a in range(3):
                        steer[a] += off[a] / max(dist, 0.5) * 0.9
            new_vel.append(steer)
        if self.falcon is not None:
            fp, fv = self.falcon
            self.falcon = ([fp[a] + fv[a] for a in range(3)], fv)
        old = self.agitation
        spread = [max(old[j] for _, j in nb[i][:7]) * 0.93 for i in range(n)]
        self.agitation = [max(old[i] * 0.86, spread[i] if spread[i] > 0.35 else 0.0) for i in range(n)]
        for i in range(n):
            steer = new_vel[i]
            if self.agitation[i] > 0.0:
                for a in range(3):
                    steer[a] += self.agitation[i] * 0.12 * rng.normal()
            v = [vel[i][a] + steer[a] for a in range(3)]
            norm = max(math.sqrt(sum(x * x for x in v)), 1e-6)
            vel[i] = [x * self.speed[i] / norm for x in v]
            for a in range(3):
                pos[i][a] += vel[i][a]

    def release_falcon(self):
        n = self.n
        centre = [sum(p[a] for p in self.pos) / n for a in range(3)]
        start = [centre[0] - 140.0, centre[1] + 40.0, centre[2] + 70.0]
        direction = [centre[a] - start[a] for a in range(3)]
        norm = math.sqrt(sum(x * x for x in direction))
        self.falcon = (start, [x / norm * 2.6 for x in direction])


def components(neighbours, k):
    """Sizes of the weakly connected components of the k-nearest-neighbour graph."""
    n = len(neighbours)
    parent = list(range(n))

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for i, row in enumerate(neighbours):
        for _, j in row[:k]:
            a, b = find(i), find(j)
            if a != b:
                parent[a] = b
    sizes = {}
    for i in range(n):
        sizes[find(i)] = sizes.get(find(i), 0) + 1
    return sorted(sizes.values(), reverse=True)


def polarisation(vel):
    n = len(vel)
    sx = sum(v[0] / math.sqrt(v[0] ** 2 + v[1] ** 2 + v[2] ** 2) for v in vel)
    sy = sum(v[1] / math.sqrt(v[0] ** 2 + v[1] ** 2 + v[2] ** 2) for v in vel)
    sz = sum(v[2] / math.sqrt(v[0] ** 2 + v[1] ** 2 + v[2] ** 2) for v in vel)
    return math.sqrt(sx * sx + sy * sy + sz * sz) / n
