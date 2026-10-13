"""The six anchors of the morph brief, simulated: Gray-Scott in pure Python.

Run from the repository root: python3 docs/figures/reaction-diffusion-patterns.py
Writes docs/media/figures/reaction-diffusion-patterns.svg (about a minute on four cores).

72 x 72 periodic dish, 5-point Laplacian, Du = 0.2097, Dv = 0.105, dt = 1, 10000 steps from
eight 4 x 4 nuclei (u, v) = (0.5, 0.25) on (u, v) = (1, 0). The map shows the activator v.
The isotop dish is 256 x 256 with zero-flux edges; the morphologies are the same.
"""

import multiprocessing

import rdsim_w8 as rd
import svgkit_w8 as kit

SIZE = 72
STEPS = 10000
SEED = 3
NUCLEI = 8
STOPS = [(0.0, (10, 14, 24)), (0.35, (20, 90, 110)), (0.7, (240, 170, 60)), (1.0, (255, 250, 220))]


def job(anchor):
    cpu, feed, kill, name = anchor
    u, v, _ = rd.run(SIZE, feed, kill, STEPS, seed=SEED, nuclei=NUCLEI)
    flat = [x for row in v for x in row]
    return v, max(flat), sum(flat) / len(flat)


def main():
    with multiprocessing.Pool(4) as pool:
        results = pool.map(job, rd.ANCHORS)
    scale, gap = 4, 22
    cell = SIZE * scale
    width = 3 * cell + 4 * gap
    height = 2 * (cell + 54) + 52
    svg = kit.Svg(width, height, "Six Gray-Scott morphologies at isotop's anchors",
                  "Simulated activator v for the six anchor points of the morph brief.")
    svg.title_text("Gray-Scott at the six morph anchors (72 x 72, 10000 steps)")
    for index, ((cpu, feed, kill, name), (v, vmax, vmean)) in enumerate(zip(rd.ANCHORS, results)):
        col, row = index % 3, index // 3
        x = gap + col * (cell + gap)
        y = 44 + row * (cell + 54)
        svg.image(x, y, cell, cell, kit.heat_png(v, STOPS, 0.0, 0.40))
        svg.rect(x, y, cell, cell, stroke=kit.AXIS)
        svg.text(x, y + cell + 17, "%s  (CPU %g%%)" % (name, cpu), size=12, weight="600")
        svg.text(x, y + cell + 33, "F = %g, k = %g   v max %.2f, mean %.3f" % (feed, kill, vmax, vmean),
                 size=10.5, fill=kit.MUTED)
        print(name, "vmax %.3f vmean %.4f" % (vmax, vmean))
    svg.save("reaction-diffusion-patterns")


if __name__ == "__main__":
    main()
