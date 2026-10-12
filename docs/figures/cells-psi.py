"""Pressure stall information: "some", "full" and the avg10 that Cells reads.

Writes docs/media/figures/cells-psi.svg. Run from the repository root:

    python3 docs/figures/cells-psi.py

Three tasks of one cgroup share a resource (memory, say). Each is productive (P), stalled on the
resource (S) or asleep and not counted (Z). PSI defines, over any window,

    some = share of wall time in which at least one task is stalled
    full = share of wall time in which every non-idle task is stalled, and at least one is

The kernel closes a 2 s period (PSI_FREQ) and folds its share into three exponential averages,
avg10, avg60 and avg300, with the fixed-point factors EXP_10s = 1677 (1 / exp(2 s / 10 s) in units
of 1/2048), EXP_60s = 1981 and EXP_300s = 2034. isotop reads only `some avg10` of
cpu.pressure, memory.pressure and io.pressure of each cgroup (`parse_pressure` in
src/platform/linux/mod.rs). The schedule below is made up; the arithmetic is the kernel's.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

END = 40.0
PERIOD = 2.0
FIXED_1 = 2048
EXP_10S = 1677

# Intervals (start, stop, state) per task.
TASKS = {
    "task A": [(0, 6, "P"), (6, 13, "S"), (13, 20, "P"), (20, 30, "S"), (30, 34, "Z"),
               (34, 40, "P")],
    "task B": [(0, 10, "P"), (10, 14, "S"), (14, 22, "Z"), (22, 28, "S"), (28, 40, "P")],
    "task C": [(0, 8, "Z"), (8, 10, "P"), (10, 13, "S"), (13, 18, "P"), (18, 24, "Z"),
               (24, 31, "S"), (31, 40, "Z")],
}
COLOUR = {"P": S.TEAL, "S": S.RED, "Z": "#2b3440"}


def state(task, t):
    for start, stop, kind in TASKS[task]:
        if start <= t < stop:
            return kind
    return "Z"


def classify(t):
    kinds = [state(task, t) for task in TASKS]
    stalled = kinds.count("S")
    awake = len(kinds) - kinds.count("Z")
    some = stalled >= 1
    full = awake >= 1 and stalled == awake
    return some, full


def main():
    step = 0.01
    some, full = [], []
    for i in range(int(END / step)):
        s, f = classify(i * step + step / 2)
        some.append(s)
        full.append(f)
    per = int(PERIOD / step)
    buckets = int(END / PERIOD)
    some_share = [sum(some[k * per:(k + 1) * per]) / per for k in range(buckets)]
    full_share = [sum(full[k * per:(k + 1) * per]) / per for k in range(buckets)]
    factor = EXP_10S / FIXED_1
    avg_some, avg_full, a, b = [], [], 0.0, 0.0
    for k in range(buckets):
        a = a * factor + some_share[k] * (1 - factor)
        b = b * factor + full_share[k] * (1 - factor)
        avg_some.append(a * 100)
        avg_full.append(b * 100)

    svg = S.Svg(900, 740, "Pressure stall information",
                "Three tasks' states, the some and full stall shares per 2 s period, and the avg10 "
                "exponential averages.")
    left, width = 90, 770
    ax = S.Axes(svg, left, 60, width, 140, (0, END), (0, 3))
    ax.frame(xticks=list(range(0, 41, 5)), yticks=[], xlabel="")
    svg.text(left, 46, "Task states: productive (teal), stalled (red), asleep (dark)", 13,
             weight="600")
    for index, task in enumerate(TASKS):
        y0 = 2 - index
        svg.text(left - 10, ax.py(y0 + 0.5) + 4, task, 11, "end", S.MUTED)
        for start, stop, kind in TASKS[task]:
            svg.rect(ax.px(start), ax.py(y0 + 0.86), ax.px(stop) - ax.px(start),
                     ax.py(y0 + 0.14) - ax.py(y0 + 0.86), COLOUR[kind], opacity=0.95)

    mid = S.Axes(svg, left, 250, width, 150, (0, END), (0, 1.0))
    mid.frame(xticks=list(range(0, 41, 5)), yticks=[0, 0.5, 1.0], xlabel="",
              ylabel="share of the 2 s period")
    svg.text(left, 238, "'some' (orange) and 'full' (red) share of each 2 s period", 13,
             weight="600")
    for k in range(buckets):
        x0, x1 = mid.px(k * PERIOD), mid.px((k + 1) * PERIOD)
        svg.rect(x0, mid.py(some_share[k]), x1 - x0, mid.py(0) - mid.py(some_share[k]),
                 S.GOLD, opacity=0.9)
        svg.rect(x0, mid.py(full_share[k]), x1 - x0, mid.py(0) - mid.py(full_share[k]),
                 S.RED, opacity=0.95)

    low = S.Axes(svg, left, 460, width, 170, (0, END), (0, 80))
    low.frame(xticks=list(range(0, 41, 5)), yticks=[0, 20, 40, 60, 80], xlabel="time (s)",
              ylabel="avg10 (%)")
    svg.text(left, 448, "avg10 as the kernel reports it (updated every 2 s) and as isotop reads it",
             13, weight="600")
    # The average that closes period k is published when the period ends, at (k + 1) * PERIOD.
    xs = [k * PERIOD for k in range(buckets + 1)]
    low.steps(xs, [0.0] + avg_some, S.GOLD, 2.2)
    low.steps(xs, [0.0] + avg_full, S.RED, 2.2)
    svg.text(low.px(31.0), low.py(avg_some[15]) - 8, "some avg10", 12, fill=S.GOLD)
    svg.text(low.px(31.0), low.py(avg_full[15]) - 8, "full avg10", 12, fill=S.RED)
    svg.text(left, 686, "A stall that lasts 10 s at 100 % reads avg10 = 100 (1 - 1/e) = 63 %: the window is "
             "a time constant, not a box of 10 s.", 11, fill=S.MUTED)
    svg.text(left, 706, "isotop samples the file every 2 s on the background thread, so a Cells "
             "membrane trembles for several seconds after the stall that caused it.", 11,
             fill=S.MUTED)
    print(svg.save("cells-psi"))
    print("some %.1f%% full %.1f%%" % (100 * sum(some) / len(some), 100 * sum(full) / len(full)))
    print("factor", factor, "e^-0.2", math.exp(-0.2), "after 5 periods", 1 - factor ** 5)
    print("peak avg10 some %.1f full %.1f" % (max(avg_some), max(avg_full)))


if __name__ == "__main__":
    main()
