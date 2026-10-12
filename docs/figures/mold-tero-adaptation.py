"""Physarum flux-reinforcement model (Tero et al. 2007, 2010) on a small mesh.

Writes docs/media/figures/mold-tero-adaptation.svg. Run from the repository root:

    python3 docs/figures/mold-tero-adaptation.py

Model. Every edge ij of a planar mesh has a length L_ij and a conductivity D_ij. For a source
node s and a sink node t that carry a total flow I0, the pressures solve Kirchhoff's law

    sum_j D_ij / L_ij (p_i - p_j) = I0 (i = s), -I0 (i = t), 0 (otherwise),

with the edge flux Q_ij = D_ij / L_ij (p_i - p_j), and the conductivity adapts as

    dD_ij / dt = f(|Q_ij|) - gamma D_ij,   f(Q) = |Q|^mu,   gamma = 1.

Top row: one fixed source-sink pair with mu = 1.3; the survivor is the shortest path, checked
against Dijkstra. Bottom row: the pair is redrawn every iteration from six food nodes, for three
values of mu. This is a small run of the model for illustration, not a reproduction of the
Tokyo experiment.
"""

import heapq
import math
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

COLS, ROWS = 9, 6
GAMMA = 1.0
STEP = 0.2
STEPS = 1500
I0 = 1.0
PRUNE = 0.02


def build_mesh():
    """A jittered triangular mesh and its edge list with Euclidean lengths."""
    rng = random.Random(20101)
    nodes = []
    for row in range(ROWS):
        for col in range(COLS):
            x = col + 0.5 * (row % 2) + rng.uniform(-0.13, 0.13)
            y = row * 0.866 + rng.uniform(-0.1, 0.1)
            nodes.append((x, y))
    edges = []
    for row in range(ROWS):
        for col in range(COLS):
            a = row * COLS + col
            if col + 1 < COLS:
                edges.append((a, a + 1))
            if row + 1 < ROWS:
                below = (row + 1) * COLS
                if row % 2 == 0:
                    if col - 1 >= 0:
                        edges.append((a, below + col - 1))
                    edges.append((a, below + col))
                else:
                    edges.append((a, below + col))
                    if col + 1 < COLS:
                        edges.append((a, below + col + 1))
    lengths = [math.dist(nodes[i], nodes[j]) for i, j in edges]
    return nodes, edges, lengths


def solve(matrix, rhs):
    """Gaussian elimination with partial pivoting, on a copy."""
    n = len(rhs)
    a = [row[:] + [rhs[i]] for i, row in enumerate(matrix)]
    for col in range(n):
        pivot = max(range(col, n), key=lambda r: abs(a[r][col]))
        a[col], a[pivot] = a[pivot], a[col]
        diag = a[col][col]
        for r in range(col + 1, n):
            factor = a[r][col] / diag
            if factor:
                row_r, row_c = a[r], a[col]
                for k in range(col, n + 1):
                    row_r[k] -= factor * row_c[k]
    x = [0.0] * n
    for r in range(n - 1, -1, -1):
        s = a[r][n] - sum(a[r][k] * x[k] for k in range(r + 1, n))
        x[r] = s / a[r][r]
    return x


def pressures(count, edges, lengths, conductivity, source, sink):
    """Kirchhoff pressures with the sink grounded at 0."""
    matrix = [[0.0] * count for _ in range(count)]
    for (i, j), length, d in zip(edges, lengths, conductivity):
        g = d / length
        matrix[i][i] += g
        matrix[j][j] += g
        matrix[i][j] -= g
        matrix[j][i] -= g
    rhs = [0.0] * count
    rhs[source] = I0
    rhs[sink] = -I0
    keep = [k for k in range(count) if k != sink]
    reduced = [[matrix[r][c] for c in keep] for r in keep]
    solution = solve(reduced, [rhs[k] for k in keep])
    p = [0.0] * count
    for index, k in enumerate(keep):
        p[k] = solution[index]
    return p


def adapt(edges, lengths, foods, mu, alternate, snapshots, seed=5):
    count = COLS * ROWS
    rng = random.Random(seed)
    d = [1.0] * len(edges)
    shots = {}
    pair = (foods[0], foods[-1])
    for step in range(STEPS + 1):
        if step in snapshots:
            shots[step] = d[:]
        if alternate:
            pair = tuple(rng.sample(foods, 2))
        p = pressures(count, edges, lengths, d, pair[0], pair[1])
        for k, ((i, j), length) in enumerate(zip(edges, lengths)):
            q = abs(d[k] / length * (p[i] - p[j]))
            d[k] += STEP * (q ** mu - GAMMA * d[k])
    return shots


def dijkstra(count, edges, lengths, source, target):
    adjacency = [[] for _ in range(count)]
    for (i, j), length in zip(edges, lengths):
        adjacency[i].append((j, length))
        adjacency[j].append((i, length))
    best = {source: 0.0}
    queue = [(0.0, source)]
    while queue:
        dist, u = heapq.heappop(queue)
        if u == target:
            return dist
        if dist > best[u]:
            continue
        for v, length in adjacency[u]:
            if dist + length < best.get(v, 1e18):
                best[v] = dist + length
                heapq.heappush(queue, (dist + length, v))
    return None


def statistics(edges, lengths, d):
    live = [k for k, value in enumerate(d) if value >= PRUNE]
    members = set()
    for k in live:
        members.update(edges[k])
    parent = {}

    def find(x):
        while parent.setdefault(x, x) != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for k in live:
        parent[find(edges[k][0])] = find(edges[k][1])
    components = len({find(x) for x in members})
    return (len(live), sum(lengths[k] for k in live), len(live) - len(members) + components)


def draw_network(svg, left, top, width, nodes, edges, d, foods, title, subtitle, scale):
    xs = [n[0] for n in nodes]
    ys = [n[1] for n in nodes]
    sx = width / (max(xs) - min(xs) + 0.6)
    ox = left + 0.3 * sx - min(xs) * sx
    oy = top + 0.3 * sx - min(ys) * sx
    height = (max(ys) - min(ys) + 0.6) * sx
    svg.rect(left, top, width, height, stroke=P.AXIS, width=1.0)
    svg.text(left, top - 24, title, size=12.5, weight="600")
    svg.text(left, top - 9, subtitle, size=11, fill=P.MUTED)
    for (i, j), value in zip(edges, d):
        if value < PRUNE:
            svg.line(ox + nodes[i][0] * sx, oy + nodes[i][1] * sx, ox + nodes[j][0] * sx,
                     oy + nodes[j][1] * sx, stroke=P.GRID, width=0.7)
    for (i, j), value in zip(edges, d):
        if value >= PRUNE:
            w = 0.7 + 6.5 * min(1.0, value / scale)
            shade = "#f2c230" if value / scale > 0.2 else "#a37a1a"
            svg.line(ox + nodes[i][0] * sx, oy + nodes[i][1] * sx, ox + nodes[j][0] * sx,
                     oy + nodes[j][1] * sx, stroke=shade, width=w, cap="round")
    for k in foods:
        svg.circle(ox + nodes[k][0] * sx, oy + nodes[k][1] * sx, 5.0, fill="#e8e0c8",
                   stroke=P.BACKGROUND, width=1.5)
    return height


def main():
    nodes, edges, lengths = build_mesh()
    count = COLS * ROWS
    foods = [COLS * 1, 7, COLS * 3 + 4, COLS * 5 + 1, COLS * 5 + 8, COLS * 2 + 8]
    shortest = dijkstra(count, edges, lengths, foods[0], foods[-1])
    early = 40
    fixed = adapt(edges, lengths, foods, 1.3, False, [0, early, STEPS])
    runs = {mu: adapt(edges, lengths, foods, mu, True, [STEPS])[STEPS] for mu in (0.8, 1.0, 1.3)}

    width, height = 1100, 600
    svg = P.Svg(width, height, "Physarum flux-reinforcement model on a small mesh",
                "Top: one fixed source-sink pair reduces a mesh to its shortest path. Bottom: a "
                "source-sink pair redrawn every iteration gives networks whose loop count falls "
                "as the exponent mu rises.")
    svg.text(24, 28, "Flux-reinforcing adaptation on a 9 x 6 jittered triangular mesh", size=16,
             weight="600")
    svg.text(24, 47, "dD/dt = |Q|^mu - D, Kirchhoff pressures re-solved every step. Line width "
             "= conductivity D; pale dots are food nodes; tubes with D < 0.02 are drawn "
             "as faint mesh.", size=11, fill=P.MUTED)
    panel, gap, left = 330, 24, 24
    scale = 0.7
    top = 100
    svg.text(left, 80, "One fixed source-sink pair, mu = 1.3", size=12.5, fill=P.ORANGE,
             weight="600")
    labels = [("t = 0", "every tube D = 1"), ("t = %d" % (early * STEP), "%d steps" % early),
              ("t = %d" % (STEPS * STEP), "%d steps" % STEPS)]
    shown = [foods[0], foods[-1]]
    height_panel = 0
    for index, step in enumerate([0, early, STEPS]):
        title, subtitle = labels[index]
        if step == STEPS:
            live, length, loops = statistics(edges, lengths, fixed[step])
            subtitle = "%d tubes, length %.2f; Dijkstra shortest path %.2f" % (live, length,
                                                                               shortest)
        height_panel = draw_network(svg, left + index * (panel + gap), top + 18, panel, nodes,
                                    edges, fixed[step], shown, title, subtitle, scale)
    top2 = top + 18 + height_panel + 75
    svg.text(left, top2 - 40, "A pair redrawn each iteration from the six food nodes "
             "(final state, 1500 steps)", size=12.5, fill=P.AQUA, weight="600")
    for index, mu in enumerate((0.8, 1.0, 1.3)):
        live, length, loops = statistics(edges, lengths, runs[mu])
        draw_network(svg, left + index * (panel + gap), top2, panel, nodes, edges, runs[mu],
                     foods, "mu = %.1f" % mu,
                     "%d tubes, length %.1f, %d independent loops" % (live, length, loops),
                     scale)
    svg.save("docs/media/figures/mold-tero-adaptation.svg")
    print("shortest path", round(shortest, 4))
    print("fixed", statistics(edges, lengths, fixed[STEPS]))
    for mu, d in runs.items():
        print("mu", mu, statistics(edges, lengths, d))


if __name__ == "__main__":
    main()
