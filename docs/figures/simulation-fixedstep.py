"""Writes docs/media/figures/simulation-fixedstep.svg: the fixed-step clock of simulation::FixedStep
with step 0.05 s: ticks, frames at 10 and 60 per second, the returned interpolation fraction, and the
rendered path of the damped oscillator of the unit test at both frame rates.

Run from the repository root: python3 docs/figures/simulation-fixedstep.py
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w3 import *  # noqa: E402,F403

H = 0.05


def alpha(t):
    return t / H - math.floor(t / H + 1e-9)


def oscillator(step, until):
    """Semi-implicit Euler of x'' = -40 x - 0.8 v from x = 1, v = 0 (the unit test's oscillator)."""
    x, v, states = 1.0, 0.0, [1.0]
    for _ in range(int(round(until / step))):
        v += (-40.0 * x - 0.8 * v) * step
        x += v * step
        states.append(x)
    return states


def main():
    svg = Svg(1100, 720, "Fixed-step clock: ticks, interpolation fraction and rendered path",
              "FixedStep with step 0.05 s. Top: the returned fraction against time with frames at 10 and 60 per second. "
              "Bottom: the damped oscillator rendered at 10 and 60 frames per second lies on one curve, while naive per-frame "
              "stepping gives a different path at each frame rate.")
    # (a) fraction
    top = Axes(svg, 70, 58, 990, 170, (0.0, 0.5), (0.0, 1.0))
    top.frame([0.0, 0.1, 0.2, 0.3, 0.4, 0.5], [0, 0.5, 1], "wall-clock time (s)", "returned fraction α", lambda v: "%g" % v, lambda v: "%g" % v)
    for k in range(11):
        svg.line(top.px(k * H), top.y, top.px(k * H), top.y + top.h, BLUE, 1, "4 3", 0.55)
    fine = [i / 4000 * 0.5 for i in range(4001)]
    saw = []
    for t in fine:
        a = alpha(t)
        if saw and a < saw[-1][1] - 0.5:
            saw.append((t - 1e-9, 1.0))
            svg.polyline([(top.px(x), top.py(y)) for x, y in saw], MUTED, 1.2)
            saw = []
        saw.append((t, a))
    svg.polyline([(top.px(x), top.py(y)) for x, y in saw], MUTED, 1.2)
    for j in range(31):
        t = j / 60
        svg.circle(top.px(t), top.py(alpha(t)), 2.2, AQUA)
    for j in range(5):
        t = 0.013 + j * 0.1
        svg.circle(top.px(t), top.py(alpha(t)), 5, "none", ORANGE, 2)
    svg.text(top.x, 18, "dashed: ticks every 0.05 s    green dots: frames at 60 per second    orange rings: frames at 10 per second", 11, MUTED)
    svg.text(top.x + 8, top.y + top.h - 8, "α = (T − T₀)/h − K  (K = ticks so far); drawn state R(T) = S[K−1] + α (S[K] − S[K−1]) = P(T − T₀ − h)", 11, INK)

    # (b) trajectory
    until = 2.0
    bot = Axes(svg, 70, 300, 990, 270, (0.0, until), (-1.15, 1.15))
    bot.frame([0, 0.5, 1.0, 1.5, 2.0], [-1, -0.5, 0, 0.5, 1], "wall-clock time (s)", "x", lambda v: "%g" % v, lambda v: "%g" % v)
    states = oscillator(H, until)

    def rendered(t):
        """P(t - h): piecewise-linear through the tick states, delayed by one step."""
        u = t - H
        if u < 0:
            return states[0]
        k = min(int(u / H), len(states) - 2)
        f = u / H - k
        return states[k] + (states[k + 1] - states[k]) * f

    naive60 = oscillator(1 / 60, until)
    naive10 = oscillator(0.1, until)
    bot.path([i / 60 for i in range(len(naive60))], naive60, AQUA, 1.4, None, 0.9)
    bot.path([i * 0.1 for i in range(len(naive10))], naive10, ORANGE, 1.6, "6 4")
    for i in range(len(naive10)):
        svg.circle(bot.px(i * 0.1), bot.py(naive10[i]), 3, ORANGE)
    ts = [i / 600 * until for i in range(601)]
    bot.path(ts, [rendered(t) for t in ts], BLUE, 2.2)
    for k, x in enumerate(states):
        svg.circle(bot.px(k * H), bot.py(x), 2.2, INK)
    for j in range(1, 21):
        t = j * 0.1 + 0.013
        svg.circle(bot.px(t), bot.py(rendered(t)), 4.5, "none", BLUE, 1.6)
    ly = bot.y + bot.h + 56
    svg.line(bot.x + 12, ly, bot.x + 40, ly, BLUE, 2.2)
    svg.text(bot.x + 46, ly + 4, "rendered with FixedStep(0.05): the 60-per-second line, with the 10-per-second frames (rings) on it", 11, INK)
    svg.circle(bot.x + 26, ly + 18, 2.2, INK)
    svg.text(bot.x + 46, ly + 22, "tick states S[k] at k·h", 11, INK)
    svg.line(bot.x + 12, ly + 40, bot.x + 40, ly + 40, AQUA, 1.4)
    svg.text(bot.x + 46, ly + 44, "naive: one Euler step per frame, 60 per second (dt = 1/60)", 11, INK)
    svg.line(bot.x + 12, ly + 58, bot.x + 40, ly + 58, ORANGE, 1.6, "6 4")
    svg.text(bot.x + 46, ly + 62, "naive: one Euler step per frame, 10 per second (dt = 0.1): a different path", 11, INK)
    svg.text(70, 690, "x'' = −40 x − 0.8 v, semi-implicit Euler (v += a h; x += v h), x(0) = 1: the oscillator of fixed_steps_follow_the_same_path_at_ten_and_sixty_frames_per_second.", 11, MUTED)
    svg.text(70, 708, "The fixed-step curve is one polyline, delayed by exactly one step (50 ms), whatever the frame rate; naive stepping drifts differently at each rate.", 11, MUTED)
    svg.save("docs/media/figures/simulation-fixedstep.svg")


main()
