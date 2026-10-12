#!/usr/bin/env python3
"""Writes docs/media/figures/theory-glow-falloff.svg: the weight profile of Item::Glow.

A glow blends its colour over what is already drawn with weight w(d) = strength * (1 - d)^2, where
d = distance / radius < 1 (Frame::halo and glow_fragment). Left: w(d) for strength 1 and the
cumulative share of the total blended weight inside radius d,
E(d) = 12 (d^2 / 2 - 2 d^3 / 3 + d^4 / 4), from the integral of (1 - d)^2 2 pi d over [0, 1] = pi / 6
(in units of radius squared). Right: the literal pixels of a glow of radius 20 px centred on a pixel
corner, sampled at pixel centres as the CPU loop does, shaded by w; the square is the box
the CPU visits.

Standard library only; run from the repository root. Deterministic.
"""

import html
import math
import pathlib

BG, FG, DIM, GRID = "#0e1116", "#c9d1d9", "#8b949e", "#30363d"
BLUE, ORANGE, GREEN, PURPLE, YELLOW, RED, TEAL = (
    "#58a6ff", "#f0883e", "#3fb950", "#bc8cff", "#d29922", "#f85149", "#39c5cf")
FONT = 'font-family="system-ui, sans-serif"'


class Svg:
    def __init__(self, width, height, title, description):
        self.width, self.height, self.title, self.description = width, height, title, description
        self.parts = []

    def text(self, x, y, s, size=12, fill=FG, anchor="start", weight="normal", style="normal", rotate=None):
        transform = f' transform="rotate({rotate} {x:.2f} {y:.2f})"' if rotate is not None else ""
        self.parts.append(
            f'<text x="{x:.2f}" y="{y:.2f}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
            f'font-weight="{weight}" font-style="{style}"{transform} {FONT}>{html.escape(s)}</text>')

    def line(self, x1, y1, x2, y2, stroke=FG, width=1.0, dash=None, opacity=1.0):
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(
            f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="{stroke}" '
            f'stroke-width="{width}" stroke-opacity="{opacity}"{extra}/>')

    def rect(self, x, y, w, h, fill="none", stroke="none", width=1.0, opacity=1.0, crisp=False):
        extra = ' shape-rendering="crispEdges"' if crisp else ""
        self.parts.append(
            f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" fill="{fill}" '
            f'fill-opacity="{opacity}" stroke="{stroke}" stroke-width="{width}"{extra}/>')

    def polyline(self, points, stroke=FG, width=1.5, dash=None):
        pts = " ".join(f"{x:.2f},{y:.2f}" for x, y in points)
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(
            f'<polyline points="{pts}" fill="none" stroke="{stroke}" stroke-width="{width}"{extra}/>')

    def circle(self, x, y, r, fill=FG, stroke="none", width=1.0):
        self.parts.append(
            f'<circle cx="{x:.2f}" cy="{y:.2f}" r="{r}" fill="{fill}" stroke="{stroke}" stroke-width="{width}"/>')

    def render(self):
        body = "\n".join(self.parts)
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" '
            f'width="{self.width}" height="{self.height}" role="img">\n'
            f"<title>{html.escape(self.title)}</title>\n<desc>{html.escape(self.description)}</desc>\n"
            f'<rect width="{self.width}" height="{self.height}" fill="{BG}"/>\n{body}\n</svg>\n')


def energy(d):
    return 12.0 * (d * d / 2 - 2 * d ** 3 / 3 + d ** 4 / 4)


def main():
    svg = Svg(960, 470, "Glow weight profile",
              "The quadratic falloff weight of a glow, the share of its blended weight inside a radius, "
              "and the literal pixels of a glow of radius 20.")
    svg.text(24, 30, "a glow is a quadratic falloff blended over the frame", 17, FG, weight="600")
    svg.text(24, 50, "Frame::halo and glow_fragment: w = strength (1 - d)^2, d = distance / radius", 12, DIM)

    # ---------------- left ----------------
    x0, x1, y0, y1 = 80, 440, 100, 340

    def X(d):
        return x0 + (x1 - x0) * d

    def Y(v):
        return y1 - (y1 - y0) * v

    svg.rect(x0, y0, x1 - x0, y1 - y0, stroke=GRID)
    for v in (0.0, 0.25, 0.5, 0.75, 1.0):
        svg.line(x0, Y(v), x1, Y(v), GRID, 1, dash="2 4")
        svg.text(x0 - 8, Y(v) + 4, f"{v:g}", 11, DIM, "end")
    for d in (0.0, 0.25, 0.5, 0.75, 1.0):
        svg.line(X(d), y1, X(d), y1 + 5, DIM, 1)
        svg.text(X(d), y1 + 19, f"{d:g}", 11, DIM, "middle")
    svg.text((x0 + x1) / 2, y1 + 38, "d = distance / radius", 12, FG, "middle")
    weight = [(X(i / 200), Y((1 - i / 200) ** 2)) for i in range(201)]
    cumulative = [(X(i / 200), Y(energy(i / 200))) for i in range(201)]
    cone = [(X(i / 200), Y(1 - i / 200)) for i in range(201)]
    svg.polyline(cone, DIM, 1.2, dash="5 4")
    svg.polyline(weight, BLUE, 2.5)
    svg.polyline(cumulative, ORANGE, 2.5)
    half = 1 - 1 / math.sqrt(2)
    svg.circle(X(half), Y(0.5), 4, BLUE)
    svg.text(X(half) + 8, Y(0.5) - 8, "w = 1/2 at d = 0.293", 11, BLUE)
    lo, hi = 0.0, 1.0
    for _ in range(60):
        mid = (lo + hi) / 2
        if energy(mid) < 0.5:
            lo = mid
        else:
            hi = mid
    svg.circle(X(lo), Y(0.5), 4, ORANGE)
    svg.text(X(lo) + 8, Y(0.5) + 18, f"half the weight lies inside d = {lo:.3f}", 11, ORANGE)
    svg.text(X(0.02) + 4, Y(0.98) - 2, "w(d) = (1 - d)^2", 12, BLUE)
    svg.text(x1 - 6, Y(0.90), "E(d), share of the total", 11.5, ORANGE, "end")
    svg.text(x1 - 6, Y(0.52), "dashed: a linear cone, for comparison", 10.5, DIM, "end")
    svg.text(x0 - 56, (y0 + y1) / 2, "weight, and cumulative share", 12, FG, "middle", rotate=-90)

    # ---------------- right: literal pixels ----------------
    radius = 20
    cell = 8
    ox, oy = 700, 235                 # the glow centre, a pixel corner
    visited = 0
    shaded = 0
    for j in range(-radius, radius + 1):
        for i in range(-radius, radius + 1):
            visited += 1
            d = math.hypot(i + 0.5, j + 0.5) / radius
            if d < 1.0:
                shaded += 1
                w = (1 - d) ** 2
                colour = tuple(int(14 + (255 - 14) * w * c) for c in (0.55, 0.78, 1.0))
                svg.rect(ox + i * cell, oy + j * cell, cell, cell, fill="#%02x%02x%02x" % colour, crisp=True)
    svg.rect(ox - radius * cell, oy - radius * cell, (2 * radius + 1) * cell, (2 * radius + 1) * cell,
             stroke=YELLOW, width=1.2)
    svg.circle(ox, oy, 2.5, FG)
    svg.text(ox, oy - radius * cell - 12, "radius 20: the box the CPU visits, and the pixels it blends", 11.5, FG, "middle")
    svg.text(ox, oy + radius * cell + 26, f"{visited} pixels visited, {shaded} blended (pi R^2 = {math.pi * 400:.0f})", 11.5, YELLOW, "middle")
    svg.text(ox, oy + radius * cell + 44, "one hypot and one three-channel blend per blended pixel", 11, DIM, "middle")

    notes = [
        "Total blended weight is strength * pi R^2 / 6: a sixth of a flat disc of the same radius. The cost is the box,",
        "between (2R + 1)^2 and (2R + 2)^2 visits, so doubling a glow's radius quadruples its CPU cost (VIEW_CONVENTIONS: cap glow radii).",
    ]
    for i, line in enumerate(notes):
        svg.text(24, 430 + 16 * i, line, 11.5, DIM)

    out = pathlib.Path("docs/media/figures/theory-glow-falloff.svg")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(svg.render())


if __name__ == "__main__":
    main()
