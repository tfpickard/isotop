"""The Matrix view's clocks: a character's flash, the release of queued lines, and the shower.

Run from the repository root: python3 docs/figures/matrix-rain-timing.py
Writes docs/media/figures/matrix-rain-timing.svg. Standard library only; the one random sample is
seeded.

Constants are those of src/matrix.rs: FLASH = 0.35 s, STEADY = 0.85, HEAD = (210, 255, 220),
UNVEIL = 3 s, the release interval 0.5 / (1 + pending / 3) seconds, SHOWER = 14 to 26 rows per
second with a lead of 1 to 11 rows above the row it decodes.
"""

import math
import os
import random
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import AQUA, BLUE, MAGENTA, MUTED, ORANGE, RED, TEXT, YELLOW, Axes, Svg, heading

FLASH, STEADY, UNVEIL = 0.35, 0.85, 3.0
HEAD = (210, 255, 220)
INFO = (80, 255, 95)


def main():
    svg = Svg(1080, 420, "Timing of the Matrix view's rain",
              "A character's colour after a stream strikes it, the interval between released "
              "journal lines against the queue length, and the time a shower takes to decode a "
              "character.")
    heading(svg, "The rain's clocks (constants of src/matrix.rs)",
            "left: one information-level character after a strike; middle: lines released per "
            "second; right: time for a shower to reach a character")
    a = Axes(svg, 60, 90, 300, 240, (0, 2.0), (0, 270), "seconds since the stream struck",
             "channel value (0 to 255)", "Flash: mix(steady, head, exp(-t / 0.35))",
             xticks=[0, 0.5, 1.0, 1.5, 2.0], yticks=[0, 64, 128, 192, 255], title_size=11.5)
    ts = [t * 0.01 for t in range(0, 201)]
    steady = [int(c * STEADY) for c in INFO]
    for k, (name, color) in enumerate((("red", RED), ("green", AQUA), ("blue", BLUE))):
        a.line(ts, [steady[k] + (HEAD[k] - steady[k]) * math.exp(-t / FLASH) for t in ts], color,
               2.2)
    a.vline(FLASH, MUTED, 1, "3 3")
    a.label(FLASH, 250, "tau = 0.35 s", 10, "start", MUTED, dx=4)
    a.legend([("red", RED, "line"), ("green", AQUA, "line"), ("blue", BLUE, "line")],
             x=a.x + 170, y=a.y + 105, size=10.5)

    b = Axes(svg, 440, 90, 270, 240, (0, 76), (0, 56), "queued lines (pending)", "lines per second",
             "Release rate = 1 / interval", xticks=[0, 16, 32, 48, 64, 76], yticks=[0, 10, 20, 30, 40, 50],
             title_size=11.5)
    ns = list(range(0, 77))
    b.line(ns, [(1 + n / 3.0) / 0.5 for n in ns], ORANGE, 2.2)
    b.label(70, 4.0, "empty: one line per 0.5 s", 9.5, "end", MUTED)
    b.label(70, 54.0, "76 queued: 52.7 lines/s", 9.5, "end", MUTED)
    b.vline(76, MUTED, 1, "3 3")

    rng = random.Random(7)
    times = [rng.uniform(1.0, 11.0) / rng.uniform(14.0, 26.0) for _ in range(20000)]
    c = Axes(svg, 790, 90, 260, 240, (0, 0.8), (0, 1.0), "seconds from release to strike",
             "share of characters struck", "Shower: lead / speed", xticks=[0, 0.2, 0.4, 0.6, 0.8],
             yticks=[0, 0.25, 0.5, 0.75, 1.0], title_size=11.5)
    times.sort()
    c.line([times[int(i * (len(times) - 1) / 200)] for i in range(201)], [i / 200 for i in range(201)],
           YELLOW, 2.2)
    median = times[len(times) // 2]
    c.hline(0.5, MUTED, 1, "3 3")
    c.label(0.8, 0.45, "median %.2f s; the last 1 percent by %.2f s" % (median, times[int(0.99 * len(times))]),
            9.5, "end", MUTED)
    svg.text(24, 408, "Unstruck characters appear anyway UNVEIL = 3 s after their line is written, so no line "
                      "stays hidden. The queue holds at most 2 x max(rows, 16) lines (76 at 38 rows).",
             10.5, "start", MUTED)
    print(svg.save("matrix-rain-timing"))
    print("median %.3f p99 %.3f max %.3f" % (median, times[int(0.99 * len(times))], times[-1]))


if __name__ == "__main__":
    main()
