"""The arbor view's specified skeleton on a seeded synthetic process tree, and why the golden angle.

Run from the repository root: python3 docs/figures/arbor-skeleton.py
Writes docs/media/figures/arbor-skeleton.svg.

The tree is not isotop output and not a real machine: 192 processes, each new one attaching to an
earlier parent with probability proportional to (children + 1)^1.2 * 0.72^depth, memory lognormal
(median 20 MiB, sigma 1.4), seeded. Geometry follows issue #54: child k of a parent takes azimuth
k * 137.5078 degrees around the parent's axis at elevation 50 degrees (children of the trunk) or 40
degrees (deeper), attaches at 0.35 + 0.6 (1 - 0.8^(k+1)) of the parent's length, and has length
14 * 0.78^depth; the radius of a branch past a junction is R_TRUNK * sqrt(A / A_trunk) with
R_TRUNK = 1.6 and A the subtree memory still carried (Leonardo's rule). Leaves, wind, gravitropism
and roots are not drawn. The reference axis for azimuth zero is this figure's choice.
"""

import math

import svgkit_w8 as kit
from izh_w8 import Normal

GOLDEN = 2.399963  # layout::GOLDEN_ANGLE, radians
R_TRUNK = 1.6


def make_tree(n=192, seed=3):
    rng = Normal(seed)
    parent, children, depth, memory = [-1], [[]], [0], [14.0]
    for i in range(1, n):
        weights = [(len(children[p]) + 1) ** 1.2 * 0.72 ** depth[p] for p in range(i)]
        x = rng.unit() * sum(weights)
        acc = 0.0
        for p, w in enumerate(weights):
            acc += w
            if x <= acc:
                break
        parent.append(p)
        children.append([])
        children[p].append(i)
        depth.append(depth[p] + 1)
        memory.append(math.exp(math.log(20.0) + 1.4 * rng.normal()))
    sub = memory[:]
    for i in range(n - 1, 0, -1):
        sub[parent[i]] += sub[i]
    return parent, children, depth, memory, sub


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def unit(v):
    n = math.sqrt(sum(x * x for x in v))
    return tuple(x / n for x in v)


def grow(parent, children, depth, memory, sub):
    n = len(parent)
    base = [None] * n
    direction = [None] * n
    length = [14.0 * 0.78 ** depth[i] for i in range(n)]
    base[0], direction[0] = (0.0, 0.0, 0.0), (0.0, 0.0, 1.0)
    for p in range(n):
        a = direction[p]
        ref = (0.0, 0.0, 1.0) if abs(a[2]) < 0.9 else (1.0, 0.0, 0.0)
        e1 = unit(cross(a, ref))
        e2 = cross(a, e1)
        theta = math.radians(50.0 if depth[p] == 0 else 40.0)
        for k, c in enumerate(children[p]):
            f = 0.35 + 0.6 * (1.0 - 0.8 ** (k + 1))
            phi = k * GOLDEN
            d = tuple(a[j] * math.cos(theta) + (e1[j] * math.cos(phi) + e2[j] * math.sin(phi))
                      * math.sin(theta) for j in range(3))
            base[c] = tuple(base[p][j] + a[j] * length[p] * f for j in range(3))
            direction[c] = unit(d)
    return base, direction, length


def max_gap(step_degrees, n):
    angles = sorted((i * step_degrees) % 360.0 for i in range(n))
    gaps = [b - a for a, b in zip(angles, angles[1:])] + [360.0 - angles[-1] + angles[0]]
    return max(gaps)


def main():
    parent, children, depth, memory, sub = make_tree()
    n = len(parent)
    base, direction, length = grow(parent, children, depth, memory, sub)
    svg = kit.Svg(960, 400, "Arbor skeleton on a seeded synthetic tree, and the golden angle",
                  "Max azimuth gap against number of slots for four turn angles, the specified "
                  "branch geometry on a synthetic 192-process tree, and child to parent radius ratios.")
    svg.title_text("arbor (planned): phyllotaxis slots, Leonardo radii, one synthetic tree")
    # a: gaps
    a = kit.Axes(svg, 62, 56, 262, 280, (1, 144), (1, 360), xlog=True, ylog=True,
                 xlabel="slots used (children of one parent)", ylabel="largest gap (degrees)",
                 title="a  azimuth gaps", xticks=[1, 3, 10, 30, 100], yticks=[1, 10, 100])
    entries = []
    ns = list(range(1, 145))
    for step, colour, name in ((137.5077640500378, kit.AQUA, "golden 137.508"),
                               (360.0 * (math.sqrt(2) - 1), kit.YELLOW, "sqrt(2) turn 149.1"),
                               (144.0, kit.ORANGE, "144 (5 spokes)"), (120.0, kit.RED, "120 (3 spokes)")):
        a.line(ns, [max_gap(step, k) for k in ns], color=colour, width=2.0)
        entries.append((name, colour, None))
    a.legend(entries, corner="bl", size=10)
    # b: skeleton
    cx, cy, scale = 480, 342, 11.5
    az, el = math.radians(28.0), math.radians(14.0)

    def project(p):
        x, y, z = p
        sx = x * math.cos(az) - y * math.sin(az)
        depth_ = x * math.sin(az) + y * math.cos(az)
        return cx + sx * scale, cy - (z * math.cos(el) + depth_ * math.sin(el)) * scale, depth_

    segments = []
    trunk_area = sub[0]
    for p in range(n):
        junctions = []
        for k, c in enumerate(children[p]):
            junctions.append((0.35 + 0.6 * (1.0 - 0.8 ** (k + 1)), c))
        junctions.sort()
        position = 0.0
        remaining = memory[p] + sum(sub[c] for c in children[p])
        for f, c in junctions + [(1.0, None)]:
            radius = max(R_TRUNK * math.sqrt(remaining / trunk_area), 0.05)
            start = tuple(base[p][j] + direction[p][j] * length[p] * position for j in range(3))
            end = tuple(base[p][j] + direction[p][j] * length[p] * f for j in range(3))
            segments.append((start, end, radius, depth[p]))
            position = f
            if c is not None:
                remaining -= sub[c]
    drawn = []
    for start, end, radius, level in segments:
        p0, p1 = project(start), project(end)
        drawn.append(((p0[2] + p1[2]) / 2.0, p0, p1, radius, level))
    drawn.sort(key=lambda item: item[0])
    svg.text(cx, 56, "b  specified geometry, synthetic tree (192 processes)", size=13, anchor="middle",
             weight="600")
    shade = ["#7f5f50", "#8d6a58", "#9c7762", "#ad8b72", "#bf9f83", "#d1b496"]
    for _, p0, p1, radius, level in drawn:
        width = max(radius * scale * 2.0 * math.cos(el), 0.8)
        svg.line(p0[0], p0[1], p1[0], p1[1], stroke=shade[min(level, 5)], width=width, cap="round")
    # c: radius ratios
    ratios = [math.sqrt(sub[i] / sub[parent[i]]) for i in range(1, n)]
    c = kit.Axes(svg, 700, 56, 230, 280, (0, 1), (0, 70), xlabel="child radius / parent radius",
                 ylabel="branches", title="c  radius ratios", xticks=[0, 0.25, 0.5, 0.75, 1.0],
                 yticks=[0, 20, 40, 60])
    bins = [0] * 20
    for r in ratios:
        bins[min(int(r * 20), 19)] += 1
    for i, count in enumerate(bins):
        left, right = c.px(i / 20.0), c.px((i + 1) / 20.0)
        top = c.py(count)
        svg.rect(left + 1, top, right - left - 2, c.py(0) - top, fill=kit.AQUA, opacity=0.8)
    c.vline(2 ** -0.5, color=kit.YELLOW)
    c.vline(2 ** (-1.0 / 3.0), color=kit.ORANGE)
    c.label(0.70, 66, "2^(-1/2)", color=kit.YELLOW, size=10, anchor="end", dx=-3)
    c.label(0.80, 58, "2^(-1/3)", color=kit.ORANGE, size=10, dx=3)
    svg.text(480, 386, "Leonardo: r_parent^2 = sum r_child^2 (+ own); Murray: r_parent^3 = sum r_child^3. "
             "Dashed: the symmetric-split ratios.", size=10.5, anchor="middle", fill=kit.MUTED)
    svg.save("arbor-skeleton")
    leaves = sum(1 for kids in children if not kids)
    print("processes %d, leaves %d, max depth %d, max fan-out %d" % (
        n, leaves, max(depth), max(len(k) for k in children)))
    print("child/parent radius: median %.3f, mean %.3f" % (sorted(ratios)[len(ratios) // 2],
                                                          sum(ratios) / len(ratios)))
    residual = max(abs(sub[i] - memory[i] - sum(sub[c] for c in children[i])) / sub[i] for i in range(n))
    print("max Leonardo residual %.2e" % residual)
    cubes = []
    for i in range(n):
        if len(children[i]) >= 2:
            cubes.append(sum(math.sqrt(sub[c]) ** 3 for c in children[i]) / math.sqrt(sub[i]) ** 3)
    cubes.sort()
    print("branching nodes %d: sum r_c^3 / r_p^3 median %.3f range %.3f to %.3f" % (
        len(cubes), cubes[len(cubes) // 2], cubes[0], cubes[-1]))
    print("max gap at 21, 55, 144 slots, golden: %.1f %.1f %.1f; 137.5 literal: %.1f %.1f %.1f" % (
        max_gap(137.5077640500378, 21), max_gap(137.5077640500378, 55), max_gap(137.5077640500378, 144),
        max_gap(137.5, 21), max_gap(137.5, 55), max_gap(137.5, 144)))
    print("137.5 literal repeats after", next(k for k in range(1, 1000) if (137.5 * k) % 360.0 == 0))


if __name__ == "__main__":
    main()
