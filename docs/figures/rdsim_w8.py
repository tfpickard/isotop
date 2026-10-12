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
