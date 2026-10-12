"""From cumulative counters to the two numbers a Cores lane shows: busy share and queue length.

Writes docs/media/figures/cores-counters.svg. Run from the repository root:

    python3 docs/figures/cores-counters.py

A made-up, fixed workload on one CPU: at each moment `n` tasks are runnable, one of them running
and `n - 1` waiting. The kernel keeps two monotone counters per CPU: the time it was not idle
(from /proc/stat, summed user, nice, system, irq, softirq and steal ticks) and the sum over
tasks of the time they waited to run (`run_delay` in /proc/schedstat). isotop reads them once per
sample and divides the change by the interval, as `platform::linux::cpus` does:

    busy = (B(t1) - B(t0)) / (t1 - t0)          in 0..1
    wait = (W(t1) - W(t0)) / (t1 - t0)          mean number of tasks waiting

The second is the mean of the integer-valued queue length, so it is not an integer.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

# (duration in seconds, tasks runnable) pairs, then idle to 12 s.
PIECES = [(0.4, 0), (0.7, 1), (0.2, 2), (0.6, 0), (0.7, 3), (0.4, 1), (1.2, 0), (0.3, 1),
          (0.5, 2), (0.4, 0), (0.6, 1), (0.4, 2), (0.5, 1), (0.9, 0), (1.1, 1), (0.6, 2),
          (0.3, 3), (0.6, 1), (0.8, 0), (0.5, 1), (0.4, 2)]
SPAN = 12.0


def workload():
    time, edges = 0.0, []
    for duration, n in PIECES:
        edges.append((time, time + duration, n))
        time += duration
    if time < SPAN:
        edges.append((time, SPAN, 0))
    return edges


def integral(edges, until, rule):
    total = 0.0
    for start, stop, n in edges:
        if start >= until:
            break
        total += rule(n) * (min(stop, until) - start)
    return total


def main():
    edges = workload()
    busy_rule = lambda n: 1.0 if n >= 1 else 0.0
    wait_rule = lambda n: float(max(n - 1, 0))
    samples = list(range(0, 13))
    B = [integral(edges, t, busy_rule) for t in samples]
    W = [integral(edges, t, wait_rule) for t in samples]
    busy = [B[k + 1] - B[k] for k in range(12)]
    wait = [W[k + 1] - W[k] for k in range(12)]

    svg = S.Svg(900, 760, "Busy share and run queue from cumulative counters",
                "A step workload on one CPU, its two kernel counters, and the per-second busy "
                "share and mean queue length derived from them.")
    left, width = 80, 780

    top = S.Axes(svg, left, 50, width, 150, (0, SPAN), (0, 3.4))
    top.frame(xticks=list(range(13)), yticks=[0, 1, 2, 3], xlabel="", ylabel="tasks runnable",
              grid=True)
    svg.text(left, 38, "1. Tasks runnable: one running (gold), the rest waiting (red)", 13,
             weight="600")
    for start, stop, n in edges:
        if n >= 1:
            svg.polygon([top.pt(start, 0), top.pt(start, 1), top.pt(stop, 1), top.pt(stop, 0)],
                        fill=S.GOLD, opacity=0.75)
        if n >= 2:
            svg.polygon([top.pt(start, 1), top.pt(start, n), top.pt(stop, n), top.pt(stop, 1)],
                        fill=S.RED, opacity=0.8)

    mid = S.Axes(svg, left, 270, width, 190, (0, SPAN), (0, 7.2))
    mid.frame(xticks=list(range(13)), yticks=[0, 2, 4, 6], xlabel="",
              ylabel="counter (seconds)")
    svg.text(left, 258, "2. The two cumulative counters: B(t) not idle, W(t) waiting", 13,
             weight="600")
    fine = [k * 0.02 for k in range(int(SPAN / 0.02) + 1)]
    mid.curve(fine, [integral(edges, t, busy_rule) for t in fine], S.GOLD, 2)
    mid.curve(fine, [integral(edges, t, wait_rule) for t in fine], S.RED, 2)
    for t, b, w in zip(samples, B, W):
        svg.circle(mid.px(t), mid.py(b), 3.2, S.GOLD, "#0e1116", 0.8)
        svg.circle(mid.px(t), mid.py(w), 3.2, S.RED, "#0e1116", 0.8)
    svg.text(mid.px(12) - 6, mid.py(B[12]) - 8, "B", 12, "end", S.GOLD, weight="600")
    svg.text(mid.px(12) - 6, mid.py(W[12]) - 8, "W", 12, "end", S.RED, weight="600")
    svg.text(mid.px(0.2), mid.py(6.6), "dots: isotop's once-per-second reads", 11, fill=S.MUTED)

    bot = S.Axes(svg, left, 530, width, 150, (0, SPAN), (0, 1.0))
    bot.frame(xticks=list(range(13)), yticks=[0, 0.5, 1.0], xlabel="time (s)",
              ylabel="busy share", yfmt=lambda v: "%g" % v)
    svg.text(left, 518, "3. What a lane shows each second: busy share (gold bars), mean queue (red)",
             13, weight="600")
    for k in range(12):
        x0, x1 = bot.px(k) + 6, bot.px(k + 1) - 6
        y = bot.py(busy[k])
        svg.rect(x0, y, (x1 - x0) / 2 - 1, bot.py(0) - y, S.GOLD, opacity=0.85)
        qy = bot.py(min(wait[k], 1.0))
        svg.rect((x0 + x1) / 2 + 1, qy, (x1 - x0) / 2 - 1, bot.py(0) - qy, S.RED, opacity=0.85)
        svg.text((x0 + x1) / 2 - 1, bot.py(0) - 6 if busy[k] < 0.12 else y - 5,
                 "%.2f" % busy[k], 9.5, "middle", S.TEXT)
        if wait[k] > 0.005:
            svg.text((x0 + x1) / 2 + 12, qy - 17 if qy > bot.top + 22 else qy + 12,
                     "%.2f" % wait[k], 9.5, "start", S.RED)
    svg.text(left + width, 718, "queue bars are clipped at 1.0 on this axis; the lane draws "
             "round(wait) red beads, at most 6", 11, "end", S.MUTED)
    svg.text(left, 740, "The sample interval sets only the time resolution: the counters never "
             "lose a burst, they lose when it happened.", 11, fill=S.MUTED)
    print(svg.save("cores-counters"))


if __name__ == "__main__":
    main()
