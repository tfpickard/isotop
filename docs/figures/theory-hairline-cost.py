#!/usr/bin/env python3
"""Writes docs/media/figures/theory-hairline-cost.svg: why Frame::hairline chops a line into pieces.

Frame::triangle visits every pixel of the triangle's bounding box (floor of the minimum, ceil of the
maximum, clamped to the frame). A line drawn as one quad of width w therefore costs the area of two
bounding boxes, which for a diagonal is quadratic in its length; Frame::hairline cuts it into pieces
of at most HAIRLINE_PIECE = 8 pixels, which makes the cost linear. This script counts the scanned
pixels exactly as the CPU loop would (two triangles per piece, corners (p+n, q+n, q-n, p-n) and the
diagonal split [0,1,2], [0,2,3]) for w = 1.1 pixels, the width the scene's links use, from an
arbitrary but fixed sub-pixel start of (0.3, 0.7).

Standard library only; run from the repository root. Deterministic.
"""

import html
import math
import pathlib

BG, FG, DIM, GRID = "#0e1116", "#c9d1d9", "#8b949e", "#30363d"
BLUE, ORANGE, GREEN, PURPLE, YELLOW, RED, TEAL = (
    "#58a6ff", "#f0883e", "#3fb950", "#bc8cff", "#d29922", "#f85149", "#39c5cf")
FONT = 'font-family="system-ui, sans-serif"'

HAIRLINE_PIECE = 8.0
WIDTH = 1.1
START = (0.3, 0.7)


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


def box(points):
    """Pixels the CPU triangle loop visits for a triangle, ignoring the frame clamp."""
    xs = [p[0] for p in points]
    ys = [p[1] for p in points]
    return (math.ceil(max(xs)) - math.floor(min(xs)) + 1) * (math.ceil(max(ys)) - math.floor(min(ys)) + 1)


def scanned(length, angle_deg, pieces):
    angle = math.radians(angle_deg)
    dx, dy = math.cos(angle) * length, math.sin(angle) * length
    nx, ny = -dy / length * WIDTH / 2, dx / length * WIDTH / 2
    total = 0
    for k in range(pieces):
        p = (START[0] + dx * k / pieces, START[1] + dy * k / pieces)
        q = (START[0] + dx * (k + 1) / pieces, START[1] + dy * (k + 1) / pieces)
        c = [(p[0] + nx, p[1] + ny), (q[0] + nx, q[1] + ny), (q[0] - nx, q[1] - ny), (p[0] - nx, p[1] - ny)]
        total += box([c[0], c[1], c[2]]) + box([c[0], c[2], c[3]])
    return total


def main():
    svg = Svg(960, 500, "Cost of a one-pixel stroke drawn as triangles",
              "Pixels scanned by the CPU triangle loop for a line of width 1.1 drawn as one quad and "
              "as pieces of at most 8 pixels, against the length of the line.")
    svg.text(24, 30, "why a hairline is cut into pieces: bounding boxes are quadratic", 17, FG, weight="600")
    svg.text(24, 50, "Frame::triangle scans the whole bounding box; Frame::hairline caps a piece at HAIRLINE_PIECE = 8 px", 12, DIM)

    x0, x1, y0, y1 = 90, 600, 90, 380
    lo_x, hi_x = 10.0, 2000.0
    lo_y, hi_y = 10.0, 3.0e6

    def X(v):
        return x0 + (x1 - x0) * (math.log10(v) - math.log10(lo_x)) / (math.log10(hi_x) - math.log10(lo_x))

    def Y(v):
        return y1 - (y1 - y0) * (math.log10(v) - math.log10(lo_y)) / (math.log10(hi_y) - math.log10(lo_y))

    svg.rect(x0, y0, x1 - x0, y1 - y0, stroke=GRID)
    for e, label in [(1, "10"), (2, "100"), (3, "1000")]:
        svg.line(X(10.0 ** e), y0, X(10.0 ** e), y1, GRID, 1, dash="2 4")
        svg.text(X(10.0 ** e), y1 + 18, label, 11, DIM, "middle")
    svg.text(X(2000.0), y1 + 18, "2000", 11, DIM, "middle")
    for e in range(1, 7):
        svg.line(x0, Y(10.0 ** e), x1, Y(10.0 ** e), GRID, 1, dash="2 4")
        svg.text(x0 - 8, Y(10.0 ** e) + 4, f"1e{e}", 11, DIM, "end")
    svg.text((x0 + x1) / 2, y1 + 38, "length of the line in pixels", 12, FG, "middle")
    svg.text(x0 - 56, (y0 + y1) / 2, "pixels scanned by the CPU", 12, FG, "middle", rotate=-90)

    lengths = [10.0 * (200.0 ** (i / 80)) for i in range(81)]
    one_diag = [(X(L), Y(scanned(L, 45.0, 1))) for L in lengths]
    cut_diag = [(X(L), Y(scanned(L, 45.0, max(1, math.ceil(L / HAIRLINE_PIECE))))) for L in lengths]
    cut_axis = [(X(L), Y(scanned(L, 0.0, max(1, math.ceil(L / HAIRLINE_PIECE))))) for L in lengths]
    area = [(X(L), Y(L * WIDTH)) for L in lengths]
    svg.polyline(one_diag, RED, 2.4)
    svg.polyline(cut_diag, ORANGE, 2.4)
    svg.polyline(cut_axis, TEAL, 2.0)
    svg.polyline(area, DIM, 1.4, dash="5 4")
    for i, (colour, label, dash) in enumerate([
        (RED, "one quad at 45 degrees: grows as L squared", None),
        (ORANGE, "pieces of at most 8 px at 45 degrees: grows as L", None),
        (TEAL, "pieces of at most 8 px, axis-aligned", None),
        (DIM, "the area actually covered, L * w", "5 4"),
    ]):
        yy = y0 + 22 + 18 * i
        svg.line(x0 + 14, yy - 4, x0 + 44, yy - 4, colour, 2.4, dash=dash)
        svg.text(x0 + 52, yy, label, 11.5, colour if colour != DIM else DIM)

    rows = [(50, 45), (100, 45), (400, 45), (1000, 45)]
    svg.text(640, 100, "scanned pixels at 45 degrees", 12.5, FG, weight="600")
    for x, label in [(690, "length"), (775, "one quad"), (860, "in pieces"), (930, "ratio")]:
        svg.text(x, 120, label, 11.5, DIM, "end")
    for i, (L, a) in enumerate(rows):
        whole = scanned(float(L), float(a), 1)
        cut = scanned(float(L), float(a), math.ceil(L / HAIRLINE_PIECE))
        for x, label in [(690, f"{L}"), (775, f"{whole:,d}"), (860, f"{cut:,d}"), (930, f"{whole / cut:.1f}x")]:
            svg.text(x, 142 + 20 * i, label, 12, FG, "end")
    svg.text(640, 242, "A piece of 8 px at 45 degrees scans", 11.5, DIM)
    svg.text(640, 258, f"{scanned(8.0, 45.0, 1)} pixels for about {8.0 * WIDTH:.0f} covered:", 11.5, DIM)
    svg.text(640, 274, "a constant factor, paid per piece.", 11.5, DIM)
    svg.text(640, 304, "CPU coverage is by pixel centre (w0, w1, w2 >= 0).", 11.5, DIM)
    svg.text(640, 320, "At 1.1 px wide, every column of a shallow stroke", 11.5, DIM)
    svg.text(640, 336, "and every row of a steep one holds a centre;", 11.5, DIM)
    svg.text(640, 352, "a stroke thinner than 1 px could skip some.", 11.5, DIM)
    svg.text(24, 470, "Counts include no frame clamp. Lines (Item::Line, Item::Beam) avoid the issue by stepping max(|dx|, |dy|) + 1 samples,", 11, DIM)
    svg.text(24, 486, "but their pixels differ between the CPU's DDA and a GPU's line rule (issue #83), which hairlines sidestep.", 11, DIM)

    out = pathlib.Path("docs/media/figures/theory-hairline-cost.svg")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(svg.render())


if __name__ == "__main__":
    main()
