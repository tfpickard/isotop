#!/usr/bin/env python3
"""Writes docs/media/figures/theory-depth-mapping.svg: how isotop's depth reaches the GPU z-buffer.

Left: the affine map of shaders.wgsl `normalized`, z = 0.01 + 0.98 (D - Dmin) / (Dmax - Dmin),
where D is Camera::view's depth (larger is nearer) and [Dmin, Dmax] is the per-frame range that
Gpu::render tracks. The dashed grey curve is what a perspective z-buffer would do for
near 1, far 50, f (d - n) / (d (f - n)); isotop does not use it, and the curve only shows what the
orthographic choice buys. Right: the smallest depth step a 32-bit float z-buffer resolves, in world
units, against the depth range of the frame, with LINE_DEPTH_BIAS (0.02) as the reference.
The step is ulp(z) * range / 0.98; ulp is 2^-24 for z in [0.5, 1) and 2^-30 near z = 0.01.

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

    def rect(self, x, y, w, h, fill="none", stroke="none", width=1.0, opacity=1.0):
        self.parts.append(
            f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" fill="{fill}" '
            f'fill-opacity="{opacity}" stroke="{stroke}" stroke-width="{width}"/>')

    def polyline(self, points, stroke=FG, width=1.5, dash=None):
        pts = " ".join(f"{x:.2f},{y:.2f}" for x, y in points)
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(
            f'<polyline points="{pts}" fill="none" stroke="{stroke}" stroke-width="{width}"{extra}/>')

    def circle(self, x, y, r, fill=FG, stroke="none"):
        self.parts.append(f'<circle cx="{x:.2f}" cy="{y:.2f}" r="{r}" fill="{fill}" stroke="{stroke}"/>')

    def render(self):
        body = "\n".join(self.parts)
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" '
            f'width="{self.width}" height="{self.height}" role="img">\n'
            f"<title>{html.escape(self.title)}</title>\n<desc>{html.escape(self.description)}</desc>\n"
            f'<rect width="{self.width}" height="{self.height}" fill="{BG}"/>\n{body}\n</svg>\n')


def main():
    svg = Svg(960, 530, "Depth mapping and resolution",
              "The affine depth map from view depth to the z-buffer, with a perspective curve for "
              "contrast, and the smallest resolvable depth step against the scene depth range.")
    svg.text(24, 30, "depth is affine and monotone: one linear map per frame", 17, FG, weight="600")
    svg.text(24, 50, "shaders.wgsl normalized(); Gpu::render tracks [Dmin, Dmax] over the solid items", 12, DIM)

    # ---------------- left panel ----------------
    x0, x1, y0, y1 = 80, 440, 90, 360

    def X(t):            # t in 0..1 along the depth range
        return x0 + (x1 - x0) * t

    def Y(z):
        return y1 - (y1 - y0) * z

    svg.rect(x0, y0, x1 - x0, y1 - y0, stroke=GRID)
    for z in (0.0, 0.25, 0.5, 0.75, 1.0):
        svg.line(x0, Y(z), x1, Y(z), GRID, 1, dash="2 4")
        svg.text(x0 - 8, Y(z) + 4, f"{z:g}", 11, DIM, "end")
    svg.rect(x0, Y(0.01), x1 - x0, Y(0.0) - Y(0.01), fill=DIM, opacity=0.18)
    svg.rect(x0, Y(1.0), x1 - x0, Y(0.99) - Y(1.0), fill=DIM, opacity=0.18)
    # affine line
    svg.line(X(0), Y(0.01), X(1), Y(0.99), BLUE, 2.5)
    svg.circle(X(0), Y(0.01), 4, BLUE)
    svg.circle(X(1), Y(0.99), 4, BLUE)
    # hypothetical perspective z-buffer, n = 1, f = 50, drawn against d from n to f on the same axis
    n, f = 1.0, 50.0
    pts = []
    for i in range(0, 201):
        t = i / 200
        d = n + (f - n) * t
        pts.append((X(t), Y(f * (d - n) / (d * (f - n)))))
    svg.polyline(pts, DIM, 1.6, dash="5 4")
    svg.text(X(0.98), Y(0.30), "dashed: a perspective z-buffer, n = 1, f = 50", 11, DIM, "end")
    svg.text(X(0.98), Y(0.30) + 14, "(illustrative; isotop has no perspective divide)", 11, DIM, "end")
    svg.text(X(0.0) + 10, Y(0.01) - 10, "farthest solid item: 0.01", 11, BLUE)
    svg.text(X(1.0) - 8, Y(0.99) + 16, "nearest: 0.99", 11, BLUE, "end")
    svg.text(X(0.14), Y(0.76), "z = 0.01 + 0.98 (D - Dmin)", 11.5, BLUE)
    svg.text(X(0.14), Y(0.76) + 15, "      / (Dmax - Dmin)", 11.5, BLUE)
    svg.text(x1 - 6, Y(0.0) - 6, "cleared to 0.0; GreaterEqual", 10.5, DIM, "end")
    for t, label in [(0, "Dmin"), (1, "Dmax")]:
        svg.text(X(t), y1 + 18, label, 12, FG, "middle")
    svg.text((x0 + x1) / 2, y1 + 38, "view depth D in world units (larger is nearer)", 12, FG, "middle")
    svg.text(x0 - 48, (y0 + y1) / 2, "depth buffer value", 12, FG, "middle", rotate=-90)

    # ---------------- right panel ----------------
    rx0, rx1, ry0, ry1 = 560, 920, 90, 360
    lo_range, hi_range = 10.0, 1.0e6
    lo_step, hi_step = 1.0e-9, 1.0

    def RX(r):
        return rx0 + (rx1 - rx0) * (math.log10(r) - math.log10(lo_range)) / (math.log10(hi_range) - math.log10(lo_range))

    def RY(v):
        return ry1 - (ry1 - ry0) * (math.log10(v) - math.log10(lo_step)) / (math.log10(hi_step) - math.log10(lo_step))

    svg.rect(rx0, ry0, rx1 - rx0, ry1 - ry0, stroke=GRID)
    for e in range(1, 7):
        x = RX(10.0 ** e)
        svg.line(x, ry0, x, ry1, GRID, 1, dash="2 4")
        svg.text(x, ry1 + 18, f"1e{e}", 11, DIM, "middle")
    for e in range(-9, 1, 3):
        y = RY(10.0 ** e)
        svg.line(rx0, y, rx1, y, GRID, 1, dash="2 4")
        svg.text(rx0 - 8, y + 4, f"1e{e}", 11, DIM, "end")
    ranges = [10.0 ** (1 + 5 * i / 100) for i in range(101)]
    high = [(RX(r), RY(2.0 ** -24 * r / 0.98)) for r in ranges]
    low = [(RX(r), RY(2.0 ** -30 * r / 0.98)) for r in ranges]
    svg.polyline(high, ORANGE, 2.2)
    svg.polyline(low, TEAL, 2.2)
    svg.line(rx0, RY(0.02), rx1, RY(0.02), GREEN, 1.8, dash="6 3")
    crossing = 0.02 * 0.98 / 2.0 ** -24
    svg.line(RX(crossing), ry0, RX(crossing), ry1, GREEN, 1, dash="2 3")
    svg.circle(RX(crossing), RY(0.02), 4, GREEN)
    svg.text(rx0 + 8, RY(0.02) - 7, "LINE_DEPTH_BIAS = 0.02", 11, GREEN)
    svg.text(RX(crossing) - 6, ry0 + 14, f"{crossing:.2e}", 10.5, GREEN, "end")
    svg.text(rx0 + 10, RY(2.0 ** -24 * 1.0e3 / 0.98) - 12, "z in [0.5, 1): ulp 2^-24", 11, ORANGE)
    svg.text(rx1 - 6, RY(3.0e-8), "z near 0.01: ulp 2^-30", 11, TEAL, "end")
    svg.text((rx0 + rx1) / 2, ry1 + 38, "depth range of the frame, Dmax - Dmin (world units)", 12, FG, "middle")
    svg.text(rx0 - 52, (ry0 + ry1) / 2, "smallest depth step (world units)", 12, FG, "middle", rotate=-90)

    notes = [
        "Left: the same affine map applies to triangles, line endpoints (plus LINE_DEPTH_BIAS), sphere surfaces",
        "(center depth + n_z * world radius) and pick discs, so every comparison is monotone in D and the CPU's raw",
        "f32 depth and the GPU's normalised depth order every pair of fragments alike, up to rounding at ties.",
        "Right: a Depth32Float buffer resolves far less than the 0.02 bias until a frame spans some 3e5 world units.",
    ]
    for i, line in enumerate(notes):
        svg.text(24, 440 + 16 * i, line, 11.5, DIM)
    svg.text(24, 520, "sources: src/shaders.wgsl (normalized), src/gpu.rs (Gpu::render), src/render/shared.rs (LINE_DEPTH_BIAS)",
             10.5, DIM, style="italic")

    out = pathlib.Path("docs/media/figures/theory-depth-mapping.svg")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(svg.render())


if __name__ == "__main__":
    main()
