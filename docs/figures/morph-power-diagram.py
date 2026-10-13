"""The morph dish's power diagram: sites on the R2 sequence, weights from memory.

Run from the repository root: python3 docs/figures/morph-power-diagram.py
Writes docs/media/figures/morph-power-diagram.svg.

Follows issue #50: seat s sits at (frac(0.5 + s * 0.7548777), frac(0.5 + s * 0.5698403)), inset
4 cells from the edge of a 256 x 256 dish; a cell belongs to the process minimising
|x - site|^2 - w with w = 0.01 * (256^2 / n) * ln(1 + MiB), ln(1 + MiB) quantised to 0.25. The
fourteen processes and their memory are the morph prototype's (docs/mockups/code/morph.py).
The inset mapping (4 + frac * 248) and rounding to nearest are this figure's reading of the issue.
"""

import math

import svgkit_w8 as kit

SIZE = 256
INSET = 4
PROCESSES = [  # name, kind, MiB (prototype order, seats 0 to 13)
    ("systemd", "system", 80), ("journald", "system", 60), ("dbus", "system", 12),
    ("postgres", "system", 900), ("nginx", "system", 140), ("kworker", "kernel", 4),
    ("ksoftirqd", "kernel", 2), ("browser", "session", 2400), ("compiler", "session", 700),
    ("language-server", "session", 1100), ("shell", "session", 9), ("redis", "container", 300),
    ("worker", "container", 450), ("tracker", "session", 60),
]
KIND = {"kernel": (124, 138, 165), "system": (46, 196, 196), "session": (242, 176, 64),
        "container": (172, 128, 242)}
ALPHA1 = 0.7548776662466927  # 1 / plastic number
ALPHA2 = 0.5698402909980532  # 1 / plastic number squared


def plastic():
    x = 1.3
    for _ in range(60):
        x = x - (x ** 3 - x - 1) / (3 * x * x - 1)
    return x


def main():
    rho = plastic()
    assert abs(1 / rho - ALPHA1) < 1e-12 and abs(1 / rho ** 2 - ALPHA2) < 1e-12
    n = len(PROCESSES)
    sites, weights = [], []
    for seat, (name, kind, mib) in enumerate(PROCESSES):
        fx = (0.5 + seat * ALPHA1) % 1.0
        fy = (0.5 + seat * ALPHA2) % 1.0
        sites.append((INSET + fx * (SIZE - 2 * INSET), INSET + fy * (SIZE - 2 * INSET)))
        quantised = round(math.log1p(mib) / 0.25) * 0.25
        weights.append(0.01 * (SIZE * SIZE / n) * quantised)
    owner = [[0] * SIZE for _ in range(SIZE)]
    counts = [0] * n
    for y in range(SIZE):
        for x in range(SIZE):
            best, who = 1e18, 0
            for i, (sx, sy) in enumerate(sites):
                d = (x + 0.5 - sx) ** 2 + (y + 0.5 - sy) ** 2 - weights[i]
                if d < best:
                    best, who = d, i
            owner[y][x] = who
            counts[who] += 1
    for i, (sx, sy) in enumerate(sites):  # every process owns at least its site cell
        cx, cy = int(sx), int(sy)
        if owner[cy][cx] != i:
            counts[owner[cy][cx]] -= 1
            owner[cy][cx] = i
            counts[i] += 1
    pixels = []
    for y in range(SIZE):
        row = []
        for x in range(SIZE):
            i = owner[y][x]
            colour = KIND[PROCESSES[i][1]]
            edge = (x and owner[y][x - 1] != i) or (y and owner[y - 1][x] != i)
            row.append(tuple(c * (0.45 if edge else 0.8) for c in colour))
        pixels.append(row)
    svg = kit.Svg(900, 450, "The power diagram of the morph dish",
                  "Patches of 14 processes on a 256 by 256 dish, and patch area against memory.")
    svg.title_text("Morph dish: weighted Voronoi (power) patches, 14 processes")
    x0, y0, side = 30, 50, 360
    svg.image(x0, y0, side, side, kit.rgb_png(pixels))
    scale = side / SIZE
    for i, ((name, kind, mib), (sx, sy)) in enumerate(zip(PROCESSES, sites)):
        svg.circle(x0 + sx * scale, y0 + sy * scale, 3, fill=kit.BACKGROUND, stroke=kit.TEXT)
        svg.text(x0 + sx * scale + 5, y0 + sy * scale - 5, name, size=9.5, fill="#ffffff")
    svg.rect(x0, y0, side, side, stroke=kit.AXIS)
    svg.text(x0, y0 + side + 18, "sites: seats 0 to 13 on the R2 sequence; colour = kind", size=10.5,
             fill=kit.MUTED)
    ax = kit.Axes(svg, 500, 60, 370, 300, (1, 3000), (0, 22), xlog=True,
                  xlabel="memory (MiB)", ylabel="share of the dish (%)",
                  title="patch area against memory")
    xs = [m for _, _, m in PROCESSES]
    ys = [100.0 * counts[i] / (SIZE * SIZE) for i in range(n)]
    for (name, kind, mib), c in zip(PROCESSES, ys):
        colour = "#%02x%02x%02x" % KIND[kind]
        ax.points([mib], [c], color=colour, r=4.5, stroke=kit.BACKGROUND)
    grid = [1.5 ** i for i in range(1, 20)]
    ax.line(grid, [100.0 / n * math.log1p(m) / (sum(math.log1p(mm) for mm in xs) / n) for m in grid],
            color=kit.MUTED, dash="5 4", width=1.3)
    ax.label(3, 20.2, "dashed: share proportional to ln(1 + MiB)", color=kit.MUTED, size=10)
    ax.label(1.3, 1.2, "equal share would be %.1f %%" % (100.0 / n), color=kit.MUTED, size=10)
    ax.hline(100.0 / n, color=kit.AXIS, width=1)
    svg.save("morph-power-diagram")
    for (name, kind, mib), (sx, sy), w, c in zip(PROCESSES, sites, weights, counts):
        print("%-16s %5d MiB  w %6.1f  site (%5.1f, %5.1f)  share %5.2f%%" % (
            name, mib, w, sx, sy, 100.0 * c / (SIZE * SIZE)))


if __name__ == "__main__":
    main()
