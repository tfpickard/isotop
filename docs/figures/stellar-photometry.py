"""Stellar photometry as implemented in src/stellar.rs: magnitudes, point spread and flash curves.

Run from the repository root: python3 docs/figures/stellar-photometry.py
Writes docs/media/figures/stellar-photometry.svg. Standard library only.

A: magnitude m = -2.5 log10(memory / 1 GiB), memory floored at 1 MiB (smaller m is brighter).
B: the halo's alpha profile strength * (1 - r / radius)^2 of a star of magnitude m in a 720-pixel
frame, from spread(): flux L = 10^(-0.4 (m - 7)), core radius (0.92 + 0.36 L^0.2), halo strength
0.07 L^0.42 (capped at 0.95, faded to 0 between m = 5.5 and 6), halo radius 1.9 + 2.9 L^0.21 (at most
6 pixels); the core is a shaded sphere of that radius (marked on the axis).
C: the nova (0.3 s rise to -4 magnitudes, then e-folding 2 s, over 6 s) and the fade (linear over
1.5 s) of flash(). D: the supernova of flash(): -6 magnitudes in 0.5 s, then the shell
1 - exp(-t / 1.1), alpha exp(-t / 1.6), warmth min(t / 1.5, 1), over 8 s (no shipped view uses it).
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import AQUA, BLUE, MAGENTA, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg, heading

MIB, GIB = 1 << 20, 1 << 30


def magnitude(memory):
    return -2.5 * math.log10(max(memory, MIB) / GIB)


def spread(m, height=720):
    scale = height / 720.0
    flux = 10.0 ** (-0.4 * (m - 7.0))
    core_px = min(max((0.92 + 0.36 * flux ** 0.2) * scale, 0.8 * scale), 6.0 * scale)
    fade = min(max((6.0 - m) / 0.5, 0.0), 1.0)
    halo = min(0.07 * flux ** 0.42, 0.95) * fade
    halo_px = min((1.9 + 2.9 * flux ** 0.21) * scale, 6.0 * scale) if halo > 0 else 0.0
    return core_px, halo_px, halo


def nova(age):
    if age > 6.0:
        return None
    return -4.0 * age / 0.3 if age < 0.3 else -4.0 * math.exp(-(age - 0.3) / 2.0)


def main():
    svg = Svg(1100, 440, "Stellar photometry in isotop",
              "Memory to magnitude, the point-spread profile of stars of different magnitude, and "
              "the nova, fade and supernova light curves.")
    heading(svg, "Memory as starlight: magnitude, point spread and flashes",
            "A: memory to magnitude; B: halo profile; C: nova and fade; D: supernova (implemented, used by no shipped view)")
    a = Axes(svg, 60, 92, 230, 260, (1, 65536), (-8.5, 4.5), "resident memory (MiB)", "magnitude (brighter up)", "A: m = -2.5 log10(M / 1 GiB)",
             xlog=True, yticks=[-8, -6, -4, -2, 0, 2, 4], ylabels=["8", "6", "4", "2", "0", "-2", "-4"], xticks=[1, 10, 100, 1000, 10000],
             xlabels=["1", "10", "100", "1 GiB", "10 GiB"], title_size=11.5)
    ms = [10 ** (k / 40.0) for k in range(0, 4 * 40 + 20)]
    a.line(ms, [-magnitude(m * MIB) for m in ms], BLUE, 2.2)
    a.hline(-6.0, MUTED, 1, "4 3")
    a.label(65000, -6.0, "halos vanish at m = 6", 9.5, "end", MUTED, dy=-5)
    for mem, name in ((1, "1 MiB: +7.5"), (10, "10 MiB: +5.0"), (1024, "1 GiB: 0"), (16384, "16 GiB: -3.0")):
        a.points([mem], [-magnitude(mem * MIB)], ORANGE, 3)
        a.label(mem, -magnitude(mem * MIB), name, 9.5, "start", TEXT, dx=6, dy=-5)

    b = Axes(svg, 360, 92, 230, 260, (0, 6), (0, 1.0), "distance from the star (pixels, 720-px frame)", "halo alpha", "B: halo profile",
             xticks=[0, 1, 2, 3, 4, 5, 6], yticks=[0, 0.25, 0.5, 0.75, 1.0], title_size=11.5)
    for m, color in ((-4, MAGENTA), (-2, ORANGE), (0, YELLOW), (2, AQUA), (4, BLUE), (5.75, VIOLET)):
        core, radius, strength = spread(m)
        rs = [r * 0.05 for r in range(0, 121)]
        b.line(rs, [strength * (1 - r / radius) ** 2 if r < radius else 0 for r in rs], color, 1.8)
        b.svg.circle(b.px(core), b.py(0), 3.2, color)
    b.legend([("m = -4, -2, 0 (capped alike)", YELLOW, "line"), ("m = 2", AQUA, "line"),
              ("m = 4", BLUE, "line"), ("m = 5.75", VIOLET, "line")], x=b.x + 60, y=b.y + 16, size=9.5)
    b.label(6, 0.0, "dots: core radius", 9.5, "end", MUTED, dy=-8)

    c = Axes(svg, 660, 92, 190, 260, (0, 6.2), (-4.4, 0.4), "age (s)", "magnitude offset  |  opacity x 4 - 4", "C: nova and fade",
             xticks=[0, 1, 2, 3, 4, 5, 6], yticks=[-4, -3, -2, -1, 0], title_size=11.5)
    ages = [k * 0.02 for k in range(0, 311)]
    c.line([t for t in ages if nova(t) is not None], [nova(t) for t in ages if nova(t) is not None], ORANGE, 2.2)
    c.line([t for t in ages if t <= 1.5], [(1 - t / 1.5) * 4 - 4 for t in ages if t <= 1.5], BLUE, 2.0, dash="5 3")
    c.label(0.35, -4.0, "peak -4 at 0.3 s", 9.5, "start", MUTED, dx=5, dy=14)
    c.label(2.3, -1.45, "e-fold 2 s", 9.5, "start", MUTED)
    c.legend([("nova (offset in magnitudes)", ORANGE, "line"), ("fade (opacity, scaled)", BLUE, "dash")], x=c.x + 30, y=c.y + 220, size=9.5)

    d = Axes(svg, 900, 92, 170, 260, (0, 8), (0, 1.05), "age (s)", "shell, alpha, warmth", "D: supernova", xticks=[0, 2, 4, 6, 8],
             yticks=[0, 0.25, 0.5, 0.75, 1.0], title_size=11.5)
    ts = [k * 0.05 for k in range(0, 161)]
    d.line(ts, [1 - math.exp(-t / 1.1) for t in ts], AQUA, 2.0)
    d.line(ts, [math.exp(-t / 1.6) for t in ts], MAGENTA, 2.0)
    d.line(ts, [min(t / 1.5, 1.0) for t in ts], ORANGE, 2.0, dash="5 3")
    d.legend([("shell radius", AQUA, "line"), ("alpha", MAGENTA, "line"), ("warmth", ORANGE, "dash")], x=d.x + 20, y=d.y + 150, size=9.5)
    svg.text(24, 410, "The flash is -6 magnitudes for the first 0.5 s (a flux 250 times the star's own), then the star is gone and only the shell remains; the nova's -4 is a flux "
                      "of 40.", 10.5, "start", MUTED)
    print(svg.save("stellar-photometry"))
    print("flux ratios: nova", 10 ** 1.6, "supernova", 10 ** 2.4)


if __name__ == "__main__":
    main()
