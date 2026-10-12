"""Stigmergy, and two transforms of the anthill view.

Writes docs/media/figures/anthill-stigmergy-crumbs.svg. Run from the repository root:

    python3 docs/figures/anthill-stigmergy-crumbs.py

Top: the double-bridge experiment (Goss et al. 1989; Deneubourg et al. 1990) in the choice model of
Deneubourg: an ant arriving at the fork picks branch A with probability (k + a)^n / ((k + a)^n +
(k + b)^n), where a and b are the amounts of pheromone laid so far on the two branches, k = 20 and
n = 2; a branch is marked only when an ant has completed its round trip. One ant arrives per time
unit; 600 ants per run, 30 runs per panel, the fraction of the last 50 ants that took branch A.
Left: two equal branches (round trip 10); the colony picks one of them at random. Right: round
trips 10 and 20; most colonies converge on the short branch, but not all.
Bottom left: the ant scale K of the view, the smallest value of 1, 2, 5, 10, 20, 50, ... such that
(syscalls per second) x (mean trip, 2.3 s) / K <= 3000 ants. Bottom right: crumb radius against
bytes per syscall, 0.55 + 0.27 log2(bytes / 64) clamped to [0.42, 3.6] pixels at 1600 px wide (the
prototype's `crumb_px`).
"""

import math
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402


def bridge(trip_a, trip_b, seed, ants=600, k=20.0, power=2.0):
    rng = random.Random(seed)
    pending = []
    pheromone = [0.0, 0.0]
    choices = []
    for i in range(ants):
        now = float(i)
        due = [p for p in pending if p[0] <= now]
        pending = [p for p in pending if p[0] > now]
        for _, branch in due:
            pheromone[branch] += 1.0
        weight_a = (k + pheromone[0]) ** power
        weight_b = (k + pheromone[1]) ** power
        branch = 0 if rng.random() < weight_a / (weight_a + weight_b) else 1
        choices.append(branch)
        pending.append((now + (trip_a if branch == 0 else trip_b), branch))
    return choices


def fraction_a(choices, window=50):
    out = []
    for i in range(len(choices)):
        part = choices[max(0, i - window + 1):i + 1]
        out.append(1.0 - sum(part) / len(part))
    return out


def ant_scale(rate, trip=2.3, cap=3000.0):
    scale = 1
    steps = [1, 2, 5]
    k = 0
    while rate * trip / scale > cap:
        k += 1
        scale = steps[k % 3] * 10 ** (k // 3)
    return scale


def crumb(nbytes):
    return min(max(0.55 + 0.27 * math.log2(max(nbytes, 1) / 64.0), 0.42), 3.6)


def main():
    svg = P.Svg(1100, 760, "Stigmergy, the ant scale K and crumb size",
                "Double-bridge experiment with equal and unequal branches, the ant scale K "
                "against the syscall rate, and the crumb radius against bytes per syscall.")
    svg.text(24, 28, "Anthill: what a pheromone trail does, and two of the view's transforms",
             size=16, weight="600")
    svg.text(24, 47, "A marked branch attracts more ants (Deneubourg model, k = 20, n = 2); K and "
             "the crumb curve are the specified mappings of the view.", size=11, fill=P.MUTED)
    left0, top0, w, h = 70, 100, 440, 230
    xs = list(range(600))
    for panel, (title, trips) in enumerate((("Two equal branches (round trip 10 and 10)", (10, 10)),
                                           ("Round trips 10 and 20: the short branch is A",
                                            (10, 20)))):
        axes = P.Axes(svg, left0 + panel * 520, top0, w, h, (0, 600), (0, 1),
                      xlabel="ants that have reached the fork" if panel == 1 else
                      "ants that have reached the fork",
                      ylabel="fraction taking A" if panel == 0 else None, title=title,
                      xticks=[0, 200, 400, 600], yticks=[0, 0.25, 0.5, 0.75, 1])
        wins = 0
        for seed in range(30):
            choices = bridge(trips[0], trips[1], seed)
            curve = fraction_a(choices)
            wins += 1 if curve[-1] > 0.5 else 0
            axes.plot(xs, curve, stroke=P.BLUE if seed % 2 == 0 else P.AQUA, width=1.0,
                      opacity=0.75)
        axes.label(595, 0.06, "%d of 30 colonies end on A" % wins, size=11, anchor="end")
        print("panel", panel, "colonies ending mostly on A:", wins)

    axes = P.Axes(svg, left0, 440, w, 230, (1e3, 1e6), (1, 1000), xlog=True, ylog=True,
                  xlabel="syscalls per second (read plus write)",
                  ylabel="K = syscalls per ant", title="Ant scale K (mean trip 2.3 s)",
                  xticks=[1e3, 1e4, 1e5, 1e6], yticks=[1, 10, 100, 1000],
                  ytick_labels=["1", "10", "100", "1000"])
    rates = [10 ** (3 + 3 * k / 400.0) for k in range(401)]
    axes.plot(rates, [ant_scale(r) for r in rates], stroke=P.YELLOW, width=2.2)
    axes.dots([57587.0], [50.0], fill=P.AQUA, radius=4)
    axes.label(57587.0, 50.0, "mockup: 57,587 calls/s, K = 50", size=10.5, dx=-8, dy=-10,
               anchor="end")

    axes = P.Axes(svg, left0 + 520, 440, w, 230, (16, 4 * 2**20), (0, 4), xlog=True,
                  xlabel="bytes per syscall (rchar / syscr)", ylabel="crumb radius (px at 1600 px)",
                  title="Crumb size", xticks=[64, 4096, 2**17, 2**20],
                  xtick_labels=["64 B", "4 KiB", "128 KiB", "1 MiB"], yticks=[0, 1, 2, 3, 4])
    sizes = [16 * 2 ** (k / 8.0) for k in range(0, 8 * 18 + 1)]
    axes.plot(sizes, [crumb(s) for s in sizes], stroke=P.ORANGE, width=2.2)
    for nbytes in (64, 4096, 2**17):
        axes.dots([nbytes], [crumb(nbytes)], fill=P.TEXT, radius=3.5)
        axes.label(nbytes, crumb(nbytes), "%.2f px" % crumb(nbytes), size=10.5, dx=6, dy=16)
    svg.save("docs/media/figures/anthill-stigmergy-crumbs.svg")
    for rate in (500, 3000, 10000, 57587, 1e6):
        print("rate", rate, "K", ant_scale(rate))


if __name__ == "__main__":
    main()
