"""Gray-Scott reaction-diffusion in pure Python (standard library only), for the wiki figures.

    u' = Du lap(u) - u v^2 + F (1 - u)
    v' = Dv lap(v) + u v^2 - (F + k) v

on an n x n periodic grid with the 5-point Laplacian, dt = 1 and Du = 0.2097, Dv = 0.105: the
lattice values the isotop brief gives for the 5-point stencil. Seeding is deterministic (a
splitmix64 stream). Used by the Math-Reaction-Diffusion and Planned-Morph figures.
"""

DU = 0.2097
DV = 0.105

MASK = (1 << 64) - 1


class Stream:
    """splitmix64, the same family as isotop's `simulation::Rng`."""

    def __init__(self, seed):
        self.state = seed & MASK

    def next_u64(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK
        return z ^ (z >> 31)

    def below(self, bound):
        return self.next_u64() % bound

    def unit(self):
        return (self.next_u64() >> 40) / 16777216.0


def new_dish(n, nuclei, seed):
    """u = 1, v = 0 with `nuclei` 4 x 4 squares of (u, v) = (0.5, 0.25), as in Pearson 1993."""
    u = [[1.0] * n for _ in range(n)]
    v = [[0.0] * n for _ in range(n)]
    stream = Stream(seed)
    for _ in range(nuclei):
        x, y = stream.below(n), stream.below(n)
        for dy in range(-2, 2):
            for dx in range(-2, 2):
                u[(y + dy) % n][(x + dx) % n] = 0.5
                v[(y + dy) % n][(x + dx) % n] = 0.25
    return u, v


def step(u, v, feed, kill, du=DU, dv=DV):
    """One explicit Euler step. `feed` and `kill` are numbers or per-cell lists of rows."""
    n = len(u)
    per_cell = not isinstance(feed, (int, float))
    nu = []
    nv = []
    for i in range(n):
        ur, um, up = u[i], u[i - 1], u[(i + 1) % n]
        vr, vm, vp = v[i], v[i - 1], v[(i + 1) % n]
        ul, uq = ur[-1:] + ur[:-1], ur[1:] + ur[:1]
        vl, vq = vr[-1:] + vr[:-1], vr[1:] + vr[:1]
        if per_cell:
            fr, kr = feed[i], kill[i]
            nu.append([a + du * (b + c + d + e - 4.0 * a) - a * g * g + f * (1.0 - a)
                       for a, b, c, d, e, g, f in zip(ur, um, up, ul, uq, vr, fr)])
            nv.append([a + dv * (b + c + d + e - 4.0 * a) + p * a * a - (f + q) * a
                       for a, b, c, d, e, p, f, q in zip(vr, vm, vp, vl, vq, ur, fr, kr)])
        else:
            nu.append([a + du * (b + c + d + e - 4.0 * a) - a * g * g + feed * (1.0 - a)
                       for a, b, c, d, e, g in zip(ur, um, up, ul, uq, vr)])
            nv.append([a + dv * (b + c + d + e - 4.0 * a) + p * a * a - (feed + kill) * a
                       for a, b, c, d, e, p in zip(vr, vm, vp, vl, vq, ur)])
    return nu, nv


def run(n, feed, kill, steps, seed=3, nuclei=6, snapshots=()):
    """Runs from a fresh dish. Returns (u, v, {step: v copy for each step in snapshots})."""
    u, v = new_dish(n, nuclei, seed)
    taken = {}
    wanted = set(snapshots)
    for s in range(1, steps + 1):
        u, v = step(u, v, feed, kill)
        if s in wanted:
            taken[s] = [row[:] for row in v]
    return u, v, taken


# ------------------------------------------------------------------------------------------
# Analysis: steady states, linearisation and the dispersion relation of Gray-Scott

import math

# The anchor table of the morph brief: (CPU percent of one core, F, k, look).
ANCHORS = [
    (0.0, 0.014, 0.054, "drifting spots"),
    (8.0, 0.030, 0.062, "solitons"),
    (30.0, 0.037, 0.060, "fingerprint"),
    (80.0, 0.029, 0.057, "maze"),
    (160.0, 0.039, 0.058, "holes"),
    (300.0, 0.018, 0.047, "turbulence"),
]


def feed_kill(cpu):
    """F and k for a CPU percentage: the anchors interpolated on log(1 + cpu), clamped at 300."""
    x = math.log1p(min(max(cpu, 0.0), 300.0))
    for (c0, f0, k0, _), (c1, f1, k1, _) in zip(ANCHORS, ANCHORS[1:]):
        a, b = math.log1p(c0), math.log1p(c1)
        if x <= b:
            t = (x - a) / (b - a)
            return f0 + (f1 - f0) * t, k0 + (k1 - k0) * t
    return ANCHORS[-1][1], ANCHORS[-1][2]


def saddle_node_k(feed):
    """The k above which no non-trivial uniform state exists: F >= 4 (F + k)^2."""
    return math.sqrt(feed) / 2.0 - feed


def hopf_feed(kill):
    """F on the Hopf curve (trace of the Jacobian zero, determinant positive), for k < 1/16."""
    root = kill * (1.0 - 4.0 * math.sqrt(kill))
    return (math.sqrt(kill) - 2.0 * kill - math.sqrt(root)) / 2.0


def steady_states(feed, kill):
    """Non-trivial uniform states (u, v), upper-u branch first; empty beyond the saddle-node."""
    disc = 1.0 - 4.0 * (feed + kill) ** 2 / feed
    if disc < 0.0:
        return []
    root = math.sqrt(disc)
    return [(u, (feed + kill) / u) for u in ((1.0 + root) / 2.0, (1.0 - root) / 2.0)]


def jacobian(feed, kill, u, v):
    """[[f_u, f_v], [g_u, g_v]] of the reaction terms at (u, v)."""
    return ((-v * v - feed, -2.0 * u * v), (v * v, 2.0 * u * v - (feed + kill)))


def growth_rate(jac, q, du=DU, dv=DV):
    """The larger real part of the eigenvalues of J - kappa D, with kappa = 2 - 2 cos q, the
    5-point Laplacian's symbol along a lattice axis."""
    kappa = 2.0 - 2.0 * math.cos(q)
    a = jac[0][0] - du * kappa
    d = jac[1][1] - dv * kappa
    trace = a + d
    det = a * d - jac[0][1] * jac[1][0]
    disc = trace * trace / 4.0 - det
    return trace / 2.0 + (math.sqrt(disc) if disc > 0.0 else 0.0)


def max_growth(jac, du=DU, dv=DV, samples=200):
    best, where = -1e9, 0.0
    for i in range(samples + 1):
        q = math.pi * i / samples
        rate = growth_rate(jac, q, du, dv)
        if rate > best:
            best, where = rate, q
    return best, where
