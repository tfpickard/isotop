#!/usr/bin/env python3
"""Seconds of past that a count of samples covers, against `--sample-ms`.

Run from the repository root: python3 docs/figures/history-window-vs-sample-ms.py
Writes docs/media/figures/history-window-vs-sample-ms.svg.

A view that asks the shared history for "the last N samples" gets N * sample_ms / 1000 seconds.
`history::CAPACITY` (240) bounds what any window can hold, and `history::SERIES` (60) is the
count Ptolemy asks for. Strata asks for a 60 second window by `Sample::elapsed`.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from sampling_plot import BLUE, GREEN, MUTED, ORANGE, PURPLE, RED, Figure  # noqa: E402

CAPACITY = 240
SERIES = 60


def main():
    figure = Figure(
        800,
        510,
        "History window against sampling interval",
        "Seconds covered by 60 and by 240 samples as --sample-ms varies, with the documented minute marked.",
    )
    figure.text(60, 28, "How much past is N samples? (issues #126 and #127)", size=15, weight="600")
    axes = figure.axes(
        72, 62, 680, 330, (100, 10000), (3, 3000), xlog=True, ylog=True,
        xlabel="--sample-ms", ylabel="seconds of history covered",
        xticks=[100, 200, 500, 1000, 2000, 5000, 10000], yticks=[5, 10, 20, 50, 100, 200, 500, 1000, 2000],
        xformat=lambda v: f"{v:g}", yformat=lambda v: f"{v:g}",
    )
    xs = [100 * 10 ** (i * 2 / 200) for i in range(201)]
    axes.path(xs, [CAPACITY * x / 1000 for x in xs], BLUE, 2.2)
    axes.path(xs, [SERIES * x / 1000 for x in xs], ORANGE, 2.2)
    axes.hline(60, GREEN, "6 4", 1.4)
    axes.note(100, 60, "the minute Strata and Ptolemy are documented to show", color=GREEN, dx=8, dy=-6)
    axes.vline(1000, MUTED, "4 4")
    axes.note(1000, 3.4, "default", color=MUTED, dx=5)
    axes.vline(250, RED, "2 4", 1.2)
    axes.note(250, 3.4, "250 ms", color=RED, dx=5)
    axes.points([100, 1000], [CAPACITY * 0.1, CAPACITY * 1.0], BLUE, 4.0)
    axes.points([100, 1000], [SERIES * 0.1, SERIES * 1.0], ORANGE, 4.0)
    axes.note(100, CAPACITY * 0.1, "24 s", color=BLUE, dx=8, dy=-8)
    axes.note(100, SERIES * 0.1, "6 s", color=ORANGE, dx=8, dy=-8)
    axes.note(1000, CAPACITY, "240 s", color=BLUE, dx=-8, dy=-10, anchor="end")
    axes.note(1000, SERIES, "60 s", color=ORANGE, dx=-8, dy=18, anchor="end")
    axes.legend(
        [(BLUE, "240 samples: the most the ring holds (CAPACITY)", None), (ORANGE, "60 samples: Ptolemy's History::recent(until, SERIES)", None)],
        x=axes.left + 330, y=axes.top + 262,
    )
    figure.text(72, 462, "Below 250 ms the ring holds under 60 s, so Strata's minute is cut short (#127).", size=11.5, color=MUTED)
    figure.text(72, 479, "Away from 1000 ms, Ptolemy's 60 samples stop being a minute (#126). A window is a duration, not a count.", size=11.5, color=MUTED)
    out = os.path.join("docs", "media", "figures", "history-window-vs-sample-ms.svg")
    figure.save(out)
    print("wrote", out)


if __name__ == "__main__":
    main()
