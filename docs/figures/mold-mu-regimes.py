"""The exponent mu in the Physarum model, on the smallest network that shows it.

Writes docs/media/figures/mold-mu-regimes.svg. Run from the repository root:

    python3 docs/figures/mold-mu-regimes.py

One source and one sink are joined by two parallel tubes of length 1 (short) and 1.5 (long),
carrying a total flow I0 = 1. The flow splits in proportion to D/L, and each conductivity obeys
dD/dt = Q^mu - D. Integrated with the explicit Euler method, h = 0.01.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

LENGTHS = (1.0, 1.5)
STEP = 0.01
END = 30.0


def run(mu, start):
    d = list(start)
    times, short, long_ = [0.0], [d[0]], [d[1]]
    for step in range(1, int(END / STEP) + 1):
        g = [d[0] / LENGTHS[0], d[1] / LENGTHS[1]]
        total = g[0] + g[1]
        q = [g[0] / total, g[1] / total]
        for i in range(2):
            d[i] += STEP * (q[i] ** mu - d[i])
        if step % 20 == 0:
            times.append(step * STEP)
            short.append(d[0])
            long_.append(d[1])
    return times, short, long_


def main():
    cases = [
        (0.6, (1.0, 1.0), "mu = 0.6: both tubes survive"),
        (1.0, (1.0, 1.0), "mu = 1: only the short tube survives"),
        (1.5, (1.0, 1.0), "mu = 1.5, equal start: the short tube wins"),
        (1.5, (0.3, 1.5), "mu = 1.5, long tube favoured at the start: it wins"),
    ]
    width, height = 1000, 640
    svg = P.Svg(width, height, "Two parallel tubes under flux reinforcement",
                "Conductivity against time for a short and a long tube between one source and "
                "one sink, for mu = 0.6, 1 and 1.5.")
    svg.text(24, 28, "Two parallel tubes, lengths 1 and 1.5, total flow 1: dD/dt = Q^mu - D",
             size=16, weight="600")
    svg.text(24, 47, "The exponent decides whether the network keeps redundancy (mu < 1), "
             "finds the shortest route (mu = 1), or commits early (mu > 1).", size=11,
             fill=P.MUTED)
    left0, top0, w, h = 70, 100, 400, 190
    for index, (mu, start, title) in enumerate(cases):
        row, col = divmod(index, 2)
        axes = P.Axes(svg, left0 + col * 470, top0 + row * 255, w, h, (0, END), (0, 1.6),
                      xlabel="time t" if row == 1 else None,
                      ylabel="conductivity D" if col == 0 else None, title=title,
                      xticks=[0, 10, 20, 30], yticks=[0, 0.5, 1.0, 1.5])
        times, short, long_ = run(mu, start)
        axes.plot(times, short, stroke=P.BLUE, width=2.2)
        axes.plot(times, long_, stroke=P.ORANGE, width=2.2)
        axes.label(END * 0.97, short[-1], "short" + (" = %.2f" % short[-1] if mu < 1 else ""),
                   anchor="end", fill=P.BLUE, dy=-7)
        axes.label(END * 0.97, long_[-1], "long" + (" = %.2f" % long_[-1] if mu < 1 else ""),
                   anchor="end", fill=P.ORANGE, dy=-7)
        print(mu, start, round(short[-1], 4), round(long_[-1], 4))
    svg.save("docs/media/figures/mold-mu-regimes.svg")


if __name__ == "__main__":
    main()
