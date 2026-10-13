"""How the City turns measurements into geometry: height from CPU, footprint from memory, and the
order in which districts fill the plane (square shells).

Writes docs/media/figures/city-encodings.svg. Run from the repository root:
    python3 docs/figures/city-encodings.py
Constants are those of Scene::draw_city and district_origin in src/render.rs.
Stdlib only, deterministic.
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w4 import *  # noqa: E402,F401,F403


def bounded(value, knee):
    return value / (value + knee)


def height(cpu):
    return 1 + 44 * bounded(cpu, 100)


def side(mib):
    return 2 * (0.42 + 1.05 * math.sqrt(bounded(mib, 220)))


def district_cell(index):
    shell = int(math.sqrt(index))
    offset = index - shell * shell
    return (shell, offset) if offset <= shell else (2 * shell - offset, shell)


figure = Figure(
    1260, 450,
    "City encodings",
    "Left: building height against CPU percent, saturating towards 45 world units with plots 4 units "
    "tall marked. Middle: building footprint side against resident memory, against what an area "
    "proportional to memory would be. Right: the order in which the first 25 districts fill square shells.",
)
figure.heading("What the City draws", "Constants of draw_city and district_origin. A plot is 4 by 4 world units.")

cpu = [i * 800 / 400 for i in range(401)]
a = Axes(figure, 70, 90, 340, 290, (0, 800), (0, 48), xlabel="CPU (percent of one core)",
         ylabel="building height (world units)", title="A. Height", xticks=[0, 100, 200, 400, 600, 800],
         yticks=[0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44])
a.hline(45, MUTED, 1.0, "4 3", "asymptote 45", 790, "end")
a.plot(cpu, [height(c) for c in cpu], BLUE, 2.4)
for value in (10, 100, 400):
    a.scatter([value], [height(value)], YELLOW, 4)
a.label(100, height(100), "one core: 23 units, 5.75 plots", 11, "start", INK, dx=10, dy=18)
a.label(400, height(400), "400%: 36", 11, "start", INK, dx=-8, dy=-12)
a.label(10, height(10), "10%: 5", 11, "start", INK, dx=8, dy=-4)

mem = [10 ** (i / 100 * 4.82) for i in range(101)]
b = Axes(figure, 500, 90, 340, 290, (1, 65536), (0, 4.4), xlog=True, xlabel="resident memory (MiB)",
         ylabel="footprint side (world units)", title="B. Footprint", xticks=[1, 10, 100, 1000, 10000],
         xformat=lambda v: "%g" % v, yticks=[0, 1, 2, 3, 4])
b.hline(4.0, MUTED, 1.0, "4 3", "plot = 4", 1.3)
b.plot(mem, [side(m) for m in mem], ORANGE, 2.4)
b.plot(mem, [side(1) * math.sqrt(m) for m in mem], MUTED, 1.4, "5 3")
b.label(1.6, 0.55, "side if area were proportional to memory", 11, "start", MUTED, dy=0)
b.label(10, 2.4, "isotop: sqrt of a saturating scale", 11, "start", ORANGE, dy=0)
for value in (1, 100, 1024):
    b.scatter([value], [side(value)], YELLOW, 4)
b.label(1024, side(1024), "1 GiB: 2.75", 11, "end", INK, dx=-8, dy=-10)

# panel C: the first 25 districts in square shells
left, top, cell = 920, 90, 56
figure.text(left, top - 10, "C. Where districts go", 13, "start", INK, weight="600")
colors = [BLUE, ORANGE, AQUA, YELLOW, MAGENTA]
for index in range(25):
    q, o = district_cell(index)
    x = left + q * cell
    y = top + 4 * cell - o * cell  # row 0 at the bottom
    shell = int(math.sqrt(index))
    figure.rect(x + 2, y + 2, cell - 4, cell - 4, colors[shell], None, 1, 0.28)
    figure.rect(x + 2, y + 2, cell - 4, cell - 4, "none", colors[shell], 1.2)
    figure.text(x + cell / 2, y + cell / 2 + 5, str(index), 14, "middle", INK, weight="600")
figure.text(left, top + 5 * cell + 20, "index = q^2 + offset; shell q = floor(sqrt(index));", 11, "start", MUTED)
figure.text(left, top + 5 * cell + 36, "shell q adds 2q+1 districts; none ever moves.", 11, "start", MUTED)
figure.text(left, top + 5 * cell + 56, "pitch 23 world units; platform 18 x 18.", 11, "start", MUTED)
figure.save(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures", "city-encodings.svg"))
