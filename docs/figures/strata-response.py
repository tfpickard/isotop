"""From a process's CPU use to a ridge: the height map and the smoothing already in the data.

Writes docs/media/figures/strata-response.svg. Run from the repository root:

    python3 docs/figures/strata-response.py

  (a) a ridge's height is `0.15 + PEAK * bounded(cpu, 60)` with PEAK = 16, where
      bounded(v, k) = v / (v + k) and `cpu` is in percent of one core (src/strata.rs). It
      saturates: 100 % is 63 % of the ceiling, 400 % is 87 %.
  (b) the history stores `Process::cpu`, which `measure` in src/model.rs has already smoothed
      with time constant 1.5 s: cpu += (1 - exp(-dt / 1.5)) (raw - cpu). At dt = 1 s the weight
      is 0.4866. A process that jumps to 100 % and stays there reads 49, 74, 87, 93 ... and a single
      second of 100 % reads 49, 25, 13, 7 ... : the area is kept, the time resolution is not.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

PEAK, KNEE, BASE = 16.0, 60.0, 0.15
TAU, DT = 1.5, 1.0


def height(cpu):
    return BASE + PEAK * cpu / (cpu + KNEE)


def main():
    svg = S.Svg(980, 420, "From CPU use to ridge height",
                "The saturating height map and the response of the smoothed CPU use to a step "
                "and to a one-second burst.")
    a = S.Axes(svg, 80, 60, 360, 270, (0, 800), (0, 17))
    a.frame(xticks=[0, 100, 200, 400, 600, 800], yticks=[0, 4, 8, 12, 16],
            xlabel="CPU use (% of one core)", ylabel="ridge height (world units)")
    svg.text(80, 46, "(a) height = 0.15 + 16 cpu / (cpu + 60)", 13, weight="600")
    xs = list(range(0, 801, 4))
    a.curve(xs, [height(x) for x in xs], S.GOLD, 2.4)
    a.curve([0, 800], [BASE + PEAK, BASE + PEAK], S.MUTED, 1, dash="4 3")
    svg.text(a.px(790), a.py(PEAK + BASE) + 14, "ceiling 16.15", 11, "end", S.MUTED)
    for cpu in (100, 400):
        svg.line(a.px(cpu), a.py(0), a.px(cpu), a.py(height(cpu)), S.TEAL, 1, dash="3 3")
        svg.circle(a.px(cpu), a.py(height(cpu)), 3.5, S.TEAL, "#0e1116", 1)
        svg.text(a.px(cpu) + 6, a.py(height(cpu)) + 16, "%d %%: %.1f" % (cpu, height(cpu)), 11,
                 fill=S.TEAL)

    alpha = 1 - math.exp(-DT / TAU)
    b = S.Axes(svg, 560, 60, 360, 270, (0, 12), (0, 100))
    b.frame(xticks=[0, 2, 4, 6, 8, 10, 12], yticks=[0, 25, 50, 75, 100], xlabel="samples after the change (1 s apart)",
            ylabel="smoothed CPU use (%)")
    svg.text(560, 46, "(b) response of the stored CPU use, tau = 1.5 s", 13, weight="600")
    step, burst, level_step, level_burst = [], [], 0.0, 0.0
    for k in range(0, 13):
        raw_step = 100.0 if k >= 1 else 0.0
        raw_burst = 100.0 if k == 1 else 0.0
        level_step += alpha * (raw_step - level_step)
        level_burst += alpha * (raw_burst - level_burst)
        step.append(level_step)
        burst.append(level_burst)
    b.curve(list(range(13)), step, S.GOLD, 2.2)
    for k, v in enumerate(step):
        svg.circle(b.px(k), b.py(v), 3.2, S.GOLD, "#0e1116", 0.8)
    for k, v in enumerate(burst):
        w = 16
        svg.rect(b.px(k) - w / 2, b.py(v), w, b.py(0) - b.py(v), S.TEAL, opacity=0.7)
    svg.text(b.px(6.4), b.py(88) - 8, "step to 100 %: 49, 74, 87, 93 ...", 11, fill=S.GOLD)
    svg.text(b.px(3.0), b.py(30), "one second at 100 %:", 11, fill=S.TEAL)
    svg.text(b.px(3.0), b.py(30) + 14, "49, 25, 13, 7 ... (sums to 100)", 11, fill=S.TEAL)
    svg.text(80, 392, "weight per sample alpha = 1 - exp(-1 / 1.5) = %.4f; the filter conserves area, so "
             "a ridge's area is its CPU time, smeared forward by about 1.5 s." % alpha, 11,
             fill=S.MUTED)
    print(svg.save("strata-response"))
    print("alpha", alpha, [round(v, 1) for v in step[:5]], [round(v, 1) for v in burst[:5]],
          "burst sum", round(sum(burst), 2))


if __name__ == "__main__":
    main()
