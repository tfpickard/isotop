"""The (F, k) plane of the Gray-Scott model with its uniform-state curves and isotop's anchors.

Run from the repository root: python3 docs/figures/reaction-diffusion-plane.py
Writes docs/media/figures/reaction-diffusion-plane.svg.

Each pixel of the shaded map is classified from the non-trivial uniform state of the model (the
lower-u branch, the one with more activator v):
  0  no non-trivial uniform state exists (F < 4 (F + k)^2 fails): only (u, v) = (1, 0) remains
  1  it exists but is unstable without diffusion (trace of the Jacobian > 0): oscillatory
  2  it is stable and stays stable under diffusion (Du = 0.2097, Dv = 0.105)
  3  it is stable without diffusion and unstable with it: a Turing instability
"""

import math

import rdsim_w8 as rd
import svgkit_w8 as kit

F_MAX = 0.08
K_MIN, K_MAX = 0.040, 0.068
COLUMNS, ROWS = 250, 200


def classify(feed, kill):
    if feed < 4.0 * (feed + kill) ** 2:
        return 0
    u, v = rd.steady_states(feed, kill)[1]
    jac = rd.jacobian(feed, kill, u, v)
    trace = jac[0][0] + jac[1][1]
    det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0]
    if det < 0.0 or trace > 0.0:
        return 1
    rate, _ = rd.max_growth(jac, samples=60)
    return 3 if rate > 1e-9 else 2


def main():
    svg = kit.Svg(900, 560, "The Gray-Scott (F, k) plane",
                  "Regions of the feed and kill plane by the fate of the non-trivial uniform "
                  "state, the saddle-node and Hopf curves, and the six isotop anchors.")
    svg.title_text("Gray-Scott (F, k) plane: where the uniform state lives, and isotop's anchors")
    x0, y0, width, height = 70, 52, 560, 416
    palette = {0: (22, 27, 34), 1: (92, 40, 52), 2: (28, 70, 58), 3: (230, 170, 40)}
    rows = []
    counts = {0: 0, 1: 0, 2: 0, 3: 0}
    for j in range(ROWS):
        kill = K_MAX - (j + 0.5) / ROWS * (K_MAX - K_MIN)
        row = bytearray()
        for i in range(COLUMNS):
            feed = (i + 0.5) / COLUMNS * F_MAX
            c = classify(feed, kill)
            counts[c] += 1
            row += bytes(palette[c])
        rows.append(bytes(row))
    svg.image(x0, y0, width, height, kit.png_bytes(COLUMNS, ROWS, rows), smooth=True)
    ax = kit.Axes(svg, x0, y0, width, height, (0.0, F_MAX), (K_MIN, K_MAX),
                  xlabel="feed rate F", ylabel="kill rate k",
                  xticks=[0, 0.02, 0.04, 0.06, 0.08], yticks=[0.04, 0.05, 0.06],
                  grid=False)
    for value in (0, 0.02, 0.04, 0.06, 0.08):
        svg.text(ax.px(value), y0 + height + 16, "%g" % value, size=11, anchor="middle", fill=kit.MUTED)
    for value in (0.04, 0.05, 0.06):
        svg.text(x0 - 7, ax.py(value) + 4, "%g" % value, size=11, anchor="end", fill=kit.MUTED)
    # saddle-node curve k = sqrt(F)/2 - F
    feeds = [i * F_MAX / 400 for i in range(1, 401)]
    ax.line(feeds, [rd.saddle_node_k(f) for f in feeds], color=kit.TEXT, width=1.8)
    # Hopf curve
    ks = [K_MIN + i * (0.0625 - K_MIN) / 300 for i in range(0, 301)]
    ax.line([rd.hopf_feed(k) for k in ks], ks, color=kit.YELLOW, width=1.8, dash="6 4")
    # the CPU path through the anchors
    path = []
    for c in range(0, 301):
        path.append(rd.feed_kill(c * 1.0))
    ax.line([p[0] for p in path], [p[1] for p in path], color=kit.AQUA, width=1.6, opacity=0.9)
    labels = {"drifting spots": (-8, -8), "solitons": (-8, -8), "fingerprint": (9, -7),
              "maze": (-9, 14), "holes": (9, 15), "turbulence": (9, 5)}
    for cpu, feed, kill, name in rd.ANCHORS:
        ax.points([feed], [kill], color=kit.AQUA, r=4.5, stroke=kit.BACKGROUND)
        dx, dy = labels[name]
        ax.label(feed, kill, "%s (%g%%)" % (name, cpu), size=10, dx=dx, dy=dy,
                 anchor="end" if dx < 0 else "start")
    # the textbook cusp
    ax.points([1 / 16], [1 / 16], color=kit.RED, r=4, stroke=kit.BACKGROUND)
    ax.label(1 / 16, 1 / 16, "F = k = 1/16", size=10, color=kit.RED, dx=7, dy=-6)
    # legend panel
    lx = x0 + width + 24
    svg.text(lx, y0 + 12, "Fate of the uniform state", size=12, weight="600")
    entries = [
        (palette[0], "only (u, v) = (1, 0)", "k above the saddle-node curve"),
        (palette[1], "non-trivial state unstable", "tr J > 0: oscillatory, not Turing"),
        (palette[2], "stable, stays stable", "stable with and without diffusion"),
        (palette[3], "Turing-unstable", "stable alone, unstable with diffusion"),
    ]
    for n, (colour, title, note) in enumerate(entries):
        y = y0 + 34 + n * 50
        svg.rect(lx, y - 12, 18, 18, fill="rgb(%d,%d,%d)" % colour, stroke=kit.AXIS)
        svg.text(lx + 26, y, title, size=11)
        svg.text(lx + 26, y + 14, note, size=10, fill=kit.MUTED)
    y = y0 + 34 + 4 * 50 + 6
    svg.line(lx, y, lx + 18, y, stroke=kit.TEXT, width=1.8)
    svg.text(lx + 26, y + 4, "saddle-node: k = sqrt(F)/2 - F", size=11)
    svg.line(lx, y + 24, lx + 18, y + 24, stroke=kit.YELLOW, width=1.8, dash="6 4")
    svg.text(lx + 26, y + 28, "Hopf: v*^2 = k", size=11)
    svg.line(lx, y + 48, lx + 18, y + 48, stroke=kit.AQUA, width=1.8)
    svg.text(lx + 26, y + 52, "morph: CPU 0 to 300 %", size=11)
    svg.text(x0 + width / 2, 520, "Du = 0.2097, Dv = 0.105 (lattice units, dt = 1), "
             "growth rates from the 5-point lattice dispersion relation", size=10,
             anchor="middle", fill=kit.MUTED)
    svg.save("reaction-diffusion-plane")
    total = sum(counts.values())
    for key in sorted(counts):
        print("class", key, "%.1f%%" % (100.0 * counts[key] / total))


if __name__ == "__main__":
    main()
