"""How the planchette glides between letters, and what the pacing rule does to a queue of lines.

Run from the repository root: python3 docs/figures/ouija-glide-queue.py
Writes docs/media/figures/ouija-glide-queue.svg.

Panels a and b: position and speed of one glide of distance D = 1 in time T = 1 for the specified
smoothstep (3u^2 - 2u^3, issue #53), the minimum-jerk polynomial (10u^3 - 15u^4 + 6u^5), a critically
damped spring tuned to reach 95 % at T (omega T = 4.744), and an underdamped spring (zeta = 0.5,
omega = 8/T, overshoot 16 %). Only the smoothstep is the specified design; the others are the
comparison models derived on the page.
Panel c: with a letter period 0.25 / (1 + q/4) s (floor 0.06 s) and q capped at 8 queued lines, a
line of L visits takes L * period(q); the equilibrium queue for an arrival rate lam is
q* = L * lam - 4 and the lines per second that must be skipped above saturation is lam - 12 / L.
"""

import math

import svgkit_w8 as kit


def smoothstep(u):
    return u * u * (3.0 - 2.0 * u)


def min_jerk(u):
    return 10 * u ** 3 - 15 * u ** 4 + 6 * u ** 5


def critical(t, omega=4.744):
    return 1.0 - (1.0 + omega * t) * math.exp(-omega * t)


def under(t, zeta=0.5, omega=8.0):
    wd = omega * math.sqrt(1 - zeta * zeta)
    return 1.0 - math.exp(-zeta * omega * t) * (math.cos(wd * t) + zeta / math.sqrt(1 - zeta * zeta)
                                                 * math.sin(wd * t))


def main():
    svg = kit.Svg(960, 352, "Planchette glide profiles and the queue pacing rule",
                  "Position and speed of four glide models, and the equilibrium queue and skip rate "
                  "of the ouija pacing rule.")
    svg.title_text("Ouija: one glide, four models; and the pacing rule under load")
    ts = [i / 400.0 for i in range(401)]
    models = [("smoothstep (specified)", kit.AQUA, lambda t: smoothstep(t)),
              ("minimum jerk", kit.YELLOW, lambda t: min_jerk(t)),
              ("critically damped spring", kit.ORANGE, critical),
              ("spring, zeta = 0.5", kit.VIOLET, under)]
    a = kit.Axes(svg, 55, 56, 260, 220, (0, 1), (0, 1.2), xlabel="t / T", title="a  position (D = 1)",
                 xticks=[0, 0.5, 1], yticks=[0, 0.5, 1])
    b = kit.Axes(svg, 365, 56, 260, 220, (0, 1), (0, 2.2), xlabel="t / T", title="b  speed (D / T)",
                 xticks=[0, 0.5, 1], yticks=[0, 1, 2])
    entries = []
    for name, colour, fn in models:
        pos = [fn(t) for t in ts]
        speed = [(pos[i + 1] - pos[i]) * 400.0 for i in range(400)]
        a.line(ts, pos, color=colour, width=2.0)
        b.line(ts[:-1], speed, color=colour, width=2.0)
        entries.append((name, colour, None))
        print("%-26s peak speed %.3f  position at T %.4f  overshoot %.4f" % (
            name, max(speed), pos[-1], max(pos) - 1.0))
    a.hline(1.0, color=kit.MUTED)
    b.legend(entries, corner="tr", size=10)
    c = kit.Axes(svg, 695, 56, 230, 220, (0, 1.0), (0, 14), xlabel="arrival rate (lines per second)",
                 title="c  queue under load", xticks=[0, 0.25, 0.5, 0.75, 1.0], yticks=[0, 4, 8, 12])
    lams = [i / 200.0 for i in range(201)]
    entries = []
    for length, colour in ((15, kit.BLUE), (30, kit.ORANGE), (40, kit.RED)):
        queue = [min(max(length * lam - 4.0, 0.0), 8.0) for lam in lams]
        skipped = [max(lam - 12.0 / length, 0.0) * length for lam in lams]  # skipped per 'L seconds'
        c.line(lams, queue, color=colour, width=2.0)
        entries.append(("L = %d visits" % length, colour, None))
    c.hline(8, color=kit.MUTED)
    c.label(0.02, 8, "queue cap 8", color=kit.MUTED, size=10, dy=-4)
    c.legend(entries, corner="br", size=10)
    svg.text(480, 342, "c: equilibrium queued lines q* = L * lam - 4, capped at 8; above lam = 12 / L "
             "lines per second the excess is skipped and counted", size=10.5, anchor="middle",
             fill=kit.MUTED)
    svg.save("ouija-glide-queue")


if __name__ == "__main__":
    main()
