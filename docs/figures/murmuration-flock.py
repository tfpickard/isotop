"""The murmuration brief's flock, ported to pure Python and run at half the prototype's size.

Run from the repository root: python3 docs/figures/murmuration-flock.py   (about 2.5 minutes)
Writes docs/media/figures/murmuration-flock.svg.

1875 birds (half of the prototype's 3750, the size the issue's connectivity test uses), the brief's
constants, 260 steps of dt = 1, then the falcon. Panels: (a) side view of the flock at step 260;
(b) the largest connected component of the 7-nearest-neighbour graph and the polarisation over time;
(c) why k = 7 alone leaves closed islands: components of the k-NN graph of 12 clumps of 8 birds;
(d) the agitation wave after the falcon is released (it arrives at step 59): birds with
agitation above 0.35 and the farthest of them from the falcon, against the flee radius 14.
"""

import math

import murm_w8 as murm
import svgkit_w8 as kit
from izh_w8 import Normal

KIND_COLOURS = ["#f2b040", "#f2b040", "#f2b040", "#f2b040", "#f2b040", "#2ec4c4", "#2ec4c4",
                "#2ec4c4", "#ac80f2", "#ac80f2", "#7c8aa5"]


def island_curve():
    rng = Normal(5)
    points, centres = [], [[14.0 * rng.normal() for _ in range(3)] for _ in range(12)]
    for centre in centres:
        for _ in range(8):
            points.append([centre[a] + rng.normal() for a in range(3)])
    flock = murm.Flock(scale=0.01)
    flock.pos = points
    flock.n = len(points)
    flock.owner = [i // 8 for i in range(flock.n)]
    neighbours = [[] for _ in range(flock.n)]
    flock.neighbours = neighbours
    flock.knn(30)
    return [(k, len(murm.components(flock.neighbours, k))) for k in range(1, 31)]


def main():
    flock = murm.Flock(scale=0.5)
    series, wave, snapshot = [], [], None
    for step in range(1, 401):
        if step == 261:
            flock.release_falcon()
        flock.step()
        if step <= 260 and step % 10 == 0:
            c7 = murm.components(flock.neighbours, 7)
            series.append((step, c7[0] / flock.n, murm.polarisation(flock.vel)))
        if step == 260:
            snapshot = [p[:] for p in flock.pos]
        if step > 260:
            falcon = flock.falcon[0]
            hit = [math.dist(flock.pos[i], falcon) for i in range(flock.n) if flock.agitation[i] > 0.35]
            wave.append((step - 260, len(hit), max(hit, default=0.0)))
    svg = kit.Svg(920, 664, "A murmuration port: connectivity, islands and the falcon's wave",
                  "Side view, connectivity and polarisation, k-NN islands and the agitation wave "
                  "of a stdlib port of the murmuration prototype at 1875 birds.")
    svg.title_text("Murmuration brief constants, simulated: %d birds, 7 topological neighbours" % flock.n)
    # (a) snapshot
    ax = kit.Axes(svg, 60, 60, 380, 230, (-70, 110), (-85, -35), title="a  side view, step 260",
                  xticks=[], yticks=[], grid=False)
    azimuth, elevation = 0.4, math.radians(18)
    cx = sum(p[0] for p in snapshot) / flock.n
    cy = sum(p[1] for p in snapshot) / flock.n
    xs, ys = [], []
    svg.group(ax.clip_name)
    for p, owner in zip(snapshot, flock.owner):
        x, y, z = p[0] - cx, p[1] - cy, p[2]
        sx = x * math.cos(azimuth) - y * math.sin(azimuth)
        depth = x * math.sin(azimuth) + y * math.cos(azimuth)
        sy = -z * math.cos(elevation) + depth * math.sin(elevation)
        svg.circle(ax.px(sx), ax.py(sy), 1.5, fill=KIND_COLOURS[owner], opacity=0.7)
    svg.end()
    # (b) connectivity
    bx = kit.Axes(svg, 530, 60, 340, 230, (0, 260), (0, 1.0), xlabel="step", title="b  one body or several?",
                  xticks=[0, 65, 130, 195, 260], yticks=[0, 0.25, 0.5, 0.75, 1.0])
    bx.line([s[0] for s in series], [s[1] for s in series], color=kit.AQUA, width=2.0)
    bx.line([s[0] for s in series], [s[2] for s in series], color=kit.ORANGE, width=2.0)
    bx.hline(0.95, color=kit.MUTED)
    bx.legend([("largest 7-NN component (fraction)", kit.AQUA, None),
               ("polarisation |sum v| / N", kit.ORANGE, None)], corner="bl")
    # (c) islands
    islands = island_curve()
    cxs = kit.Axes(svg, 60, 360, 380, 220, (1, 30), (0, 14), xlabel="k (neighbours per bird)",
                   title="c  closed islands: 12 clumps of 8 birds", xticks=[1, 7, 8, 15, 22, 30],
                   yticks=[0, 4, 8, 12])
    cxs.step([k for k, _ in islands], [c for _, c in islands], color=kit.YELLOW, width=2.0)
    cxs.points([k for k, _ in islands], [c for _, c in islands], color=kit.YELLOW, r=3)
    cxs.vline(7, color=kit.MUTED)
    cxs.label(7.3, 12.6, "k = 7: 12 islands", size=10, color=kit.MUTED)
    # (d) wave
    dx = kit.Axes(svg, 530, 360, 340, 220, (0, 140), (0, 100), xlabel="steps after release",
                  title="d  agitation wave after the falcon arrives",
                  xticks=[0, 35, 70, 105, 140], yticks=[0, 25, 50, 75, 100])
    dx.line([w[0] for w in wave], [100.0 * w[1] / flock.n for w in wave], color=kit.BLUE, width=2.0)
    dx.line([w[0] for w in wave], [w[2] for w in wave], color=kit.ORANGE, width=2.0)
    dx.hline(14, color=kit.MUTED)
    dx.label(2, 14, "flee radius 14", size=10, color=kit.MUTED, dy=-4)
    svg.text(700, 652, "blue: percent of the flock above 0.35; orange: farthest such bird from the falcon (world units)", size=10,
             anchor="middle", fill=kit.MUTED)
    svg.save("murmuration-flock")
    peak = max(wave, key=lambda w: w[1])
    print("peak agitated %d of %d at step +%d; farthest %.1f" % (peak[1], flock.n, peak[0],
                                                                max(w[2] for w in wave)))
    print("component fractions:", [(s, round(f, 3)) for s, f, _ in series])
    print("islands:", islands)


if __name__ == "__main__":
    main()
