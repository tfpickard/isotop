"""Writes docs/media/figures/colour-palette.svg: the palette of src/palette.rs, the same swatches under
three colour-vision-deficiency simulations (Machado et al. 2009, severity 1.0, applied in linear RGB;
matrix entries transcribed by the author, each row sums to 1), and `mix` (a lerp of encoded bytes, as the
code does) against a linear-light blend, with the relative luminance of each.

Run from the repository root: python3 docs/figures/colour-palette.py
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from figkit_w3 import *  # noqa: E402,F403

PALETTE = [("KERNEL", (104, 122, 156)), ("SYSTEM", (72, 183, 199)), ("SESSION", (236, 178, 92)),
           ("CONTAINER", (178, 136, 232)), ("ZOMBIE", (213, 97, 126)), ("STOPPED", (225, 161, 75)),
           ("BLOCKED", (222, 94, 73)), ("UNKNOWN_CORE", (140, 160, 196))]
MACHADO = {
    "protanopia": [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
    "deuteranopia": [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]],
    "tritanopia": [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900]],
}


def to_lin(c):
    c /= 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def to_enc(v):
    v = min(max(v, 0.0), 1.0)
    return 12.92 * v if v <= 0.0031308 else 1.055 * v ** (1 / 2.4) - 0.055


def luminance(rgb):
    r, g, b = (to_lin(c) for c in rgb)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def simulate(rgb, kind):
    lin = [to_lin(c) for c in rgb]
    m = MACHADO[kind]
    return tuple(round(255 * to_enc(sum(m[i][j] * lin[j] for j in range(3)))) for i in range(3))


def mix(a, b, t):
    """palette::mix: per channel (a + (b - a) * t) as u8, truncating."""
    return tuple(int(a[k] + (b[k] - a[k]) * t) for k in range(3))


def linear_blend(a, b, t):
    return tuple(round(255 * to_enc((1 - t) * to_lin(a[k]) + t * to_lin(b[k]))) for k in range(3))


def tint(c, s):
    return tuple(int(min(max(v * s, 0.0), 255.0)) for v in c)


def hexc(rgb):
    return "#%02x%02x%02x" % tuple(rgb)


def main():
    svg = Svg(1100, 820, "The isotop palette under colour-vision deficiency, and mix against linear light",
              "Eight palette colours for normal vision and for protanopia, deuteranopia and tritanopia, then the mix of SYSTEM and "
              "SESSION as the code computes it (a lerp of encoded bytes) against the linear-light blend, with relative luminance.")
    x0, cw, ch = 150, 118, 50
    rows = [("normal vision", None), ("protanopia", "protanopia"), ("deuteranopia", "deuteranopia"), ("tritanopia", "tritanopia")]
    for i, (name, rgb) in enumerate(PALETTE):
        svg.text(x0 + i * cw + (cw - 6) / 2, 28, name, 11, INK, "middle", weight="600")
        svg.text(x0 + i * cw + (cw - 6) / 2, 44, hexc(rgb), 10, MUTED, "middle")
    for r, (label, kind) in enumerate(rows):
        y = 56 + r * (ch + 26)
        svg.text(20, y + ch / 2 + 4, label, 12, INK)
        for i, (name, rgb) in enumerate(PALETTE):
            shown = rgb if kind is None else simulate(rgb, kind)
            svg.rect(x0 + i * cw, y, cw - 6, ch, hexc(shown), None)
            if kind is None:
                svg.text(x0 + i * cw + (cw - 6) / 2, y + ch + 11, "Y %.3f" % luminance(rgb), 10, MUTED, "middle")
    yb = 56 + 4 * (ch + 26) + 4
    svg.text(20, yb + 4, "closest pairs, ΔE00:  normal SESSION–STOPPED 4.5   deuteranopia SYSTEM–UNKNOWN_CORE 1.6, SESSION–STOPPED 3.8   tritanopia ZOMBIE–BLOCKED 4.5", 11, MUTED)

    # strips
    ys = yb + 30
    a, b = PALETTE[1][1], PALETTE[2][1]
    steps, sw = 36, 20
    svg.text(20, ys + 22, "mix(SYSTEM, SESSION, t)", 11, INK)
    svg.text(20, ys + 36, "encoded bytes (the code)", 10, MUTED)
    svg.text(20, ys + 22 + 44, "linear-light blend", 11, INK)
    svg.text(20, ys + 36 + 44, "(for comparison)", 10, MUTED)
    svg.text(20, ys + 22 + 88, "tint(SESSION, s)", 11, INK)
    svg.text(20, ys + 36 + 88, "s = 0.25 .. 1.2", 10, MUTED)
    for k in range(steps):
        t = k / (steps - 1)
        svg.rect(x0 + k * sw, ys, sw, 38, hexc(mix(a, b, t)))
        svg.rect(x0 + k * sw, ys + 44, sw, 38, hexc(linear_blend(a, b, t)))
        s = 0.25 + (1.2 - 0.25) * k / (steps - 1)
        svg.rect(x0 + k * sw, ys + 88, sw, 38, hexc(tint(PALETTE[2][1], s)))
    svg.text(x0 + steps * sw + 12, ys + 24, "t = 0.5: (154, 180, 145), Y 0.416", 11, INK)
    svg.text(x0 + steps * sw + 12, ys + 68, "t = 0.5: (179, 181, 157), Y 0.451", 11, INK)
    svg.text(x0 + steps * sw + 12, ys + 112, "Y ~ s^2.2 until R clips at 255", 11, INK)

    # luminance curves
    py = ys + 196
    for p, (title, c0, c1, xo) in enumerate([("SYSTEM → SESSION", a, b, 70), ("white → black", (255, 255, 255), (0, 0, 0), 600)]):
        lo, hi = (min(luminance(c0), luminance(c1)), max(luminance(c0), luminance(c1)))
        ax = Axes(svg, xo, py, 430, 150, (0.0, 1.0), (max(0, lo - 0.03), hi + 0.03))
        ax.frame([0, 0.25, 0.5, 0.75, 1], [round(lo, 2), round((lo + hi) / 2, 2), round(hi, 2)], "t", "relative luminance Y", lambda v: "%g" % v, lambda v: "%g" % v)
        ts = [i / 100 for i in range(101)]
        ax.path(ts, [luminance(mix(c0, c1, t)) for t in ts], ORANGE, 2.2)
        ax.path(ts, [luminance(linear_blend(c0, c1, t)) for t in ts], AQUA, 2.2, "7 4")
        svg.text(xo, py - 40, title, 12, INK, weight="600")
        svg.text(xo, py - 24, "orange: mix of encoded bytes", 11, ORANGE)
        svg.text(xo, py - 8, "green dashed: linear-light blend", 11, AQUA)
    svg.save("docs/media/figures/colour-palette.svg")


main()
