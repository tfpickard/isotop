"""How far a marble's angle drifts from the true cumulative CPU time.

Writes docs/media/figures/cores-phase-lag.svg. Run from the repository root:

    python3 docs/figures/cores-phase-lag.py

A marble in src/cores.rs starts at angle 2 pi cpu_time / LAP and then adds
2 pi / LAP * (cpu / 100) * dt every frame, where `cpu` is the scene's eased copy of the model's
smoothed CPU use. Two exponential filters stand between the kernel's counter and that angle:

  1. `measure` in src/model.rs, once per sample:  cpu += (1 - exp(-dt / 1.5)) * (raw - cpu)
  2. `Scene::ease` in src/render.rs, every frame: cpu += (1 - exp(-dt / 0.3)) * (held - cpu)

For a filter y' = (x - y) / tau the area between input and output is tau * (y(t) - y(0)), so the
integral of the output differs from the integral of the input by about tau times the current
level. This script runs both filters over a made-up workload (idle, 100 %, 50 %, idle) sampled
once a second, drawn at 20 frames a second, and measures the difference.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

LAP = 10.0
SAMPLE = 1.0
FRAME = 0.05
END = 20.0
TAU_MODEL, TAU_SCENE = 1.5, 0.3


def raw_percent(k):
    """CPU use over the interval ending at sample k (percent of one core)."""
    t = k * SAMPLE
    if t <= 2:
        return 0.0
    if t <= 8:
        return 100.0
    if t <= 14:
        return 50.0
    return 0.0


def main():
    samples = int(END / SAMPLE)
    raw = [raw_percent(k) for k in range(samples + 1)]
    smoothed = [0.0]
    for k in range(1, samples + 1):
        alpha = 1 - math.exp(-SAMPLE / TAU_MODEL)
        smoothed.append(smoothed[-1] + alpha * (raw[k] - smoothed[-1]))
    true_time = [0.0]
    for k in range(1, samples + 1):
        true_time.append(true_time[-1] + raw[k] / 100.0 * SAMPLE)

    times, eased, angle_seconds, lag = [], [], [], []
    e, marble = 0.0, 0.0
    steps = int(END / FRAME)
    for i in range(steps + 1):
        t = i * FRAME
        k = min(int(t / SAMPLE + 1e-9), samples)
        if i > 0:
            e += (1 - math.exp(-FRAME / TAU_SCENE)) * (smoothed[k] - e)
            marble += e / 100.0 * FRAME
        times.append(t)
        eased.append(e)
        angle_seconds.append(marble)
        # True cumulative CPU time at time t: constant use inside each interval, so linear.
        ahead = raw[min(k + 1, samples)] / 100.0 * (t - k * SAMPLE) if k < samples else 0.0
        lag.append(true_time[k] + ahead - marble)
    peak = max(lag)
    where = times[lag.index(peak)]

    svg = S.Svg(900, 660, "Phase lag of a marble",
                "Raw, smoothed and eased CPU use for a step workload, and the difference between "
                "the cumulative CPU time and the marble's angle.")
    left, width = 80, 780
    top = S.Axes(svg, left, 50, width, 230, (0, END), (0, 110))
    top.frame(xticks=list(range(0, 21, 2)), yticks=[0, 50, 100], xlabel="",
              ylabel="CPU use (% of one core)")
    svg.text(left, 38, "Input and the two filters", 13, weight="600")
    top.steps([k * SAMPLE for k in range(samples)], raw[1:], S.MUTED, 1.6)
    top.curve([samples * SAMPLE - SAMPLE, samples * SAMPLE], [raw[samples]] * 2, S.MUTED, 1.6)
    top.curve([k * SAMPLE for k in range(samples + 1)], smoothed, S.GOLD, 1.0, dash="4 3")
    for k in range(samples + 1):
        svg.circle(top.px(k * SAMPLE), top.py(smoothed[k]), 2.8, S.GOLD, "#0e1116", 0.6)
    top.curve(times, eased, S.TEAL, 2.0)
    svg.text(top.px(8.4), top.py(100) - 8, "raw CPU of each 1 s interval (steps)", 11, fill=S.MUTED)
    svg.text(top.px(2.2), top.py(30), "model smoothing, tau 1.5 s (dots, per sample)", 11,
             fill=S.GOLD)
    svg.text(top.px(2.2), top.py(18), "scene easing, tau 0.3 s (line, per frame)", 11, fill=S.TEAL)

    bot = S.Axes(svg, left, 350, width, 200, (0, END), (-0.4, 2.4))
    bot.frame(xticks=list(range(0, 21, 2)), yticks=[0, 1, 2], xlabel="time (s)",
              ylabel="lag (CPU-seconds)")
    svg.text(left, 338, "Cumulative CPU time minus the marble's angle, in CPU-seconds", 13,
             weight="600")
    bot.curve(times, lag, S.VIOLET, 2.0)
    svg.line(bot.px(0), bot.py(0), bot.px(END), bot.py(0), S.AXIS, 1)
    svg.text(bot.px(where) + 8, bot.py(peak) - 6,
             "peak %.2f s = %.3f lap (at t = %.1f s)" % (peak, peak / LAP, where), 11, fill=S.VIOLET)
    svg.text(left, 618, "One lap is 10 CPU-seconds, so 1 CPU-second of lag is 0.1 lap. The lag follows "
             "the current CPU level and returns to zero when the process idles;", 11, fill=S.MUTED)
    svg.text(left, 636, "it never accumulates, because the filters conserve area. The marble is "
             "re-seated at 2 pi cpu_time / LAP whenever it is created again.", 11, fill=S.MUTED)
    print(svg.save("cores-phase-lag"))
    print("peak lag", round(peak, 3), "at", round(where, 2), "final lag", round(lag[-1], 4))


if __name__ == "__main__":
    main()
