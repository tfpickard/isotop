#!/usr/bin/env python3
"""Writes docs/media/figures/theory-isometric-axes.svg: isotop's camera as an orthographic map.

Left: a unit cube drawn with Camera::view (src/render.rs) at the three pitches that matter,
LOWEST_PITCH (0.35 rad), ISOMETRIC (asin(1/sqrt 3) = 0.615480 rad) and top-down (pi/2), with the
default rotation of 0. Right: the on-screen length per world unit of the world z axis
(SCALE * cos pitch) and of the ground axis that points into the screen (SCALE * sin pitch), with
SCALE = sqrt(3/2) = 1.224745 (src/render.rs).

Standard library only; run from the repository root. Deterministic.
"""

import html
import math
import pathlib

BG, FG, DIM, GRID = "#0e1116", "#c9d1d9", "#8b949e", "#30363d"
BLUE, ORANGE, GREEN, PURPLE, YELLOW, RED, TEAL = (
    "#58a6ff", "#f0883e", "#3fb950", "#bc8cff", "#d29922", "#f85149", "#39c5cf")
FONT = 'font-family="system-ui, sans-serif"'

SCALE = 1.224_745
ISOMETRIC = 0.615_480
LOWEST_PITCH = 0.35


class Svg:
    def __init__(self, width, height, title, description):
        self.width, self.height, self.title, self.description = width, height, title, description
        self.parts = []

    def text(self, x, y, s, size=12, fill=FG, anchor="start", weight="normal", style="normal"):
        self.parts.append(
            f'<text x="{x:.2f}" y="{y:.2f}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
            f'font-weight="{weight}" font-style="{style}" {FONT}>{html.escape(s)}</text>')

    def line(self, x1, y1, x2, y2, stroke=FG, width=1.0, dash=None, opacity=1.0):
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(
            f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="{stroke}" '
            f'stroke-width="{width}" stroke-opacity="{opacity}"{extra}/>')

    def rect(self, x, y, w, h, fill="none", stroke="none", width=1.0, opacity=1.0):
        self.parts.append(
            f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" fill="{fill}" '
            f'fill-opacity="{opacity}" stroke="{stroke}" stroke-width="{width}"/>')

    def polygon(self, points, fill="none", stroke="none", width=1.0, opacity=1.0):
        pts = " ".join(f"{x:.2f},{y:.2f}" for x, y in points)
        self.parts.append(
            f'<polygon points="{pts}" fill="{fill}" fill-opacity="{opacity}" stroke="{stroke}" '
            f'stroke-width="{width}" stroke-linejoin="round"/>')

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


def view(p, rotation, pitch):
    """Camera::view with the centre at the origin: screen x, screen y (down), depth (nearer is larger)."""
    s, c = math.sin(rotation + math.pi / 4), math.cos(rotation + math.pi / 4)
    across = p[0] * c - p[1] * s
    along = p[0] * s + p[1] * c
    ps, pc = math.sin(pitch), math.cos(pitch)
    return (across * SCALE, (along * ps - p[2] * pc) * SCALE, along * pc + p[2] * ps)


def cube(svg, ox, oy, unit, pitch, label, sub):
    """Draws the three faces of the unit cube that face the viewer."""
    def at(x, y, z):
        v = view((x - 0.5, y - 0.5, z - 0.5), 0.0, pitch)
        return (ox + v[0] * unit, oy + v[1] * unit)

    top = [at(0, 0, 1), at(1, 0, 1), at(1, 1, 1), at(0, 1, 1)]
    face_x = [at(1, 0, 0), at(1, 1, 0), at(1, 1, 1), at(1, 0, 1)]   # normal +x
    face_y = [at(0, 1, 0), at(1, 1, 0), at(1, 1, 1), at(0, 1, 1)]   # normal +y
    svg.polygon(face_x, fill=BLUE, stroke=FG, width=1.2, opacity=0.55)
    svg.polygon(face_y, fill=TEAL, stroke=FG, width=1.2, opacity=0.40)
    svg.polygon(top, fill=YELLOW, stroke=FG, width=1.2, opacity=0.55)
    svg.text(ox, oy + unit * 1.55 + 34, label, 13, FG, "middle", weight="600")
    svg.text(ox, oy + unit * 1.55 + 51, sub, 11, DIM, "middle")


def main():
    svg = Svg(960, 470, "Isotop's camera is orthographic",
              "Unit cubes projected at three pitches, and the on-screen length of the world z axis "
              "and of the ground depth axis as a function of pitch.")
    svg.text(24, 30, "the camera is an axonometric projection: no perspective divide", 17, FG, weight="600")
    svg.text(24, 50, "Camera::view and Frame::project; screen units are frame pixels at zoom 1", 12, DIM)

    # ---- left: three cubes ----
    unit = 62
    cy = 190
    for ox, pitch, label, sub in [
        (95, LOWEST_PITCH, "pitch 0.35 rad (20.1 deg)", "LOWEST_PITCH"),
        (255, ISOMETRIC, "pitch 0.6155 rad (35.3 deg)", "ISOMETRIC, the default"),
        (415, math.pi / 2, "pitch pi/2 (90 deg)", "top-down, key t"),
    ]:
        cube(svg, ox, cy, unit, pitch, label, sub)
    svg.text(24, 400, "faces drawn: top (+z), +x and +y, the three with a positive dot product against", 11.5, DIM)
    svg.text(24, 416, "the viewer vector (s cos p, c cos p, sin p). At the isometric pitch each axis is", 11.5, DIM)
    svg.text(24, 432, "one screen unit long; x and y lie at 30 deg below the horizontal.", 11.5, DIM)

    # ---- right: foreshortening curves ----
    px0, px1, py0, py1 = 590, 925, 90, 330      # plot box
    ymax = 1.4

    def X(pitch):
        return px0 + (px1 - px0) * pitch / (math.pi / 2)

    def Y(v):
        return py1 - (py1 - py0) * v / ymax

    svg.rect(px0, py0, px1 - px0, py1 - py0, stroke=GRID)
    for v in (0.0, 0.5, 1.0):
        svg.line(px0, Y(v), px1, Y(v), GRID, 1, dash="2 4")
        svg.text(px0 - 8, Y(v) + 4, f"{v:g}", 11, DIM, "end")
    for deg in (0, 15, 30, 45, 60, 75, 90):
        x = X(math.radians(deg))
        svg.line(x, py1, x, py1 + 5, DIM, 1)
        svg.text(x, py1 + 19, f"{deg}", 11, DIM, "middle")
    svg.text((px0 + px1) / 2, py1 + 38, "pitch (degrees)", 12, FG, "middle")
    svg.text(px0 - 44, (py0 + py1) / 2, "screen length", 12, FG, "middle")
    # rotate label by wrapping in a transform
    svg.parts[-1] = (
        f'<text transform="translate({px0 - 40},{(py0 + py1) / 2}) rotate(-90)" font-size="12" fill="{FG}" '
        f'text-anchor="middle" {FONT}>screen length per world unit</text>')
    n = 120
    cos_curve = [(X(math.pi / 2 * i / n), Y(SCALE * math.cos(math.pi / 2 * i / n))) for i in range(n + 1)]
    sin_curve = [(X(math.pi / 2 * i / n), Y(SCALE * math.sin(math.pi / 2 * i / n))) for i in range(n + 1)]
    svg.line(px0, Y(SCALE), px1, Y(SCALE), GREEN, 2)
    svg.polyline(cos_curve, BLUE, 2.2)
    svg.polyline(sin_curve, ORANGE, 2.2)
    for pitch, name in [(LOWEST_PITCH, "LOWEST_PITCH"), (ISOMETRIC, "ISOMETRIC")]:
        x = X(pitch)
        svg.line(x, py0, x, py1, DIM, 1, dash="3 3")
        svg.text(x + 3, py0 + 12, name, 10.5, DIM)
    svg.circle(X(ISOMETRIC), Y(SCALE * math.cos(ISOMETRIC)), 4, FG)
    svg.text(px1 - 4, Y(SCALE) - 6, "across axis: SCALE = 1.2247, constant", 11, GREEN, "end")
    svg.text(px0 + 8, Y(SCALE * math.cos(0.12)) + 16, "world z: SCALE cos(pitch)", 11, BLUE)
    svg.text(px1 - 4, Y(SCALE * math.sin(1.2)) + 20, "ground depth: SCALE sin(pitch)", 11, ORANGE, "end")
    svg.text(px0, py1 + 64, "At ISOMETRIC both curves cross 1.000 (z) and 0.707 (depth): the three world axes", 11, DIM)
    svg.text(px0, py1 + 80, "are equally foreshortened, which the factor SCALE = sqrt(3/2) restores to length 1.", 11, DIM)
    svg.text(24, 458, "source: src/render.rs (Camera::view, SCALE, ISOMETRIC, LOWEST_PITCH)", 10.5, DIM, style="italic")

    out = pathlib.Path("docs/media/figures/theory-isometric-axes.svg")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(svg.render())


if __name__ == "__main__":
    main()
