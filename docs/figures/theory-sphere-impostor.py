#!/usr/bin/env python3
"""Writes docs/media/figures/theory-sphere-impostor.svg: how Frame::ball shades a sphere impostor.

Left: the geometry in the plane y = cy. A pixel at x sees the normalised offset n_x = (x - cx) / r;
the orthographic ray through it meets the sphere where n_z = sqrt(1 - n_x^2 - n_y^2) and the
surface depth is cz + n_z R. Right: the literal pixels of a sphere of radius r = 20 px, sampled at
integer coordinates as Frame::ball does, shaded with the shared constants of src/render/shared.rs
(SPHERE_AMBIENT 0.25, light (-0.4, -0.5, 0.65), cap 1.2) for a base colour of (100, 160, 210).
The ellipse is the terminator n . L = 0; below it a pixel is at the ambient floor.

Standard library only; run from the repository root. Deterministic.
"""

import html
import math
import pathlib

BG, FG, DIM, GRID = "#0e1116", "#c9d1d9", "#8b949e", "#30363d"
BLUE, ORANGE, GREEN, PURPLE, YELLOW, RED, TEAL = (
    "#58a6ff", "#f0883e", "#3fb950", "#bc8cff", "#d29922", "#f85149", "#39c5cf")
FONT = 'font-family="system-ui, sans-serif"'

AMBIENT, LX, LY, LZ, LIGHT_MAX = 0.25, 0.4, 0.5, 0.65, 1.2


class Svg:
    def __init__(self, width, height, title, description):
        self.width, self.height, self.title, self.description = width, height, title, description
        self.parts = []

    def text(self, x, y, s, size=12, fill=FG, anchor="start", weight="normal", style="normal", rotate=None):
        transform = f' transform="rotate({rotate} {x:.2f} {y:.2f})"' if rotate is not None else ""
        self.parts.append(
            f'<text x="{x:.2f}" y="{y:.2f}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
            f'font-weight="{weight}" font-style="{style}"{transform} {FONT}>{html.escape(s)}</text>')

    def line(self, x1, y1, x2, y2, stroke=FG, width=1.0, dash=None, opacity=1.0, arrow=False):
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        extra += ' marker-end="url(#arrow-%s)"' % stroke.strip("#") if arrow else ""
        self.parts.append(
            f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="{stroke}" '
            f'stroke-width="{width}" stroke-opacity="{opacity}"{extra}/>')

    def rect(self, x, y, w, h, fill="none", stroke="none", width=1.0, opacity=1.0):
        self.parts.append(
            f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" fill="{fill}" '
            f'fill-opacity="{opacity}" stroke="{stroke}" stroke-width="{width}" shape-rendering="crispEdges"/>')

    def polyline(self, points, stroke=FG, width=1.5, dash=None):
        pts = " ".join(f"{x:.2f},{y:.2f}" for x, y in points)
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        self.parts.append(
            f'<polyline points="{pts}" fill="none" stroke="{stroke}" stroke-width="{width}"{extra}/>')

    def circle(self, x, y, r, fill=FG, stroke="none", width=1.0):
        self.parts.append(
            f'<circle cx="{x:.2f}" cy="{y:.2f}" r="{r}" fill="{fill}" stroke="{stroke}" stroke-width="{width}"/>')

    def render(self, arrows=()):
        defs = "".join(
            f'<marker id="arrow-{c.strip("#")}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
            f'markerHeight="7" orient="auto"><path d="M0 0L10 5L0 10z" fill="{c}"/></marker>' for c in arrows)
        body = "\n".join(self.parts)
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" '
            f'width="{self.width}" height="{self.height}" role="img">\n'
            f"<title>{html.escape(self.title)}</title>\n<desc>{html.escape(self.description)}</desc>\n"
            f'<defs>{defs}</defs>\n<rect width="{self.width}" height="{self.height}" fill="{BG}"/>\n{body}\n</svg>\n')


def light(nx, ny):
    """The shading factor of Frame::ball for a normalised offset inside the unit disc."""
    nz = math.sqrt(max(0.0, 1.0 - nx * nx - ny * ny))
    return min(AMBIENT + max(-nx * LX - ny * LY + nz * LZ, 0.0), LIGHT_MAX)


def main():
    svg = Svg(960, 560, "Sphere impostor geometry and shading",
              "Ray-sphere geometry of a shaded impostor and the literal pixels of a 20 pixel sphere "
              "shaded by the implemented light.")
    svg.text(24, 30, "a sphere is a disc plus an analytic normal", 17, FG, weight="600")
    svg.text(24, 50, "Frame::ball and ball_fragment: no mesh, one square root per covered pixel", 12, DIM)

    # ---------------- left: geometry in the plane y = cy ----------------
    cx, cy, R = 215, 285, 140            # centre of the slice, display radius
    svg.text(24, 82, "slice through the centre (n_y = 0)", 12.5, FG, weight="600")
    # depth axis and screen axis
    svg.line(cx - R - 30, cy, cx + R + 30, cy, DIM, 1)
    svg.line(cx, cy + 22, cx, cy - R - 38, DIM, 1)
    svg.text(cx + R + 30, cy + 16, "screen x", 11, DIM, "end")
    svg.text(cx + 8, cy - R - 28, "depth, towards the viewer", 11, DIM)
    # front surface arc
    arc = [(cx + R * math.cos(math.pi - math.pi * i / 120), cy - R * math.sin(math.pi - math.pi * i / 120))
           for i in range(121)]
    svg.polyline(arc, BLUE, 2.5)
    # rays and hits
    for nx in (-0.85, -0.5, -0.15, 0.15, 0.5, 0.85):
        top = cy - R * math.sqrt(1 - nx * nx)
        svg.line(cx + nx * R, cy - R - 20, cx + nx * R, top, ORANGE, 1.1, dash="3 3", opacity=0.8)
        svg.circle(cx + nx * R, top, 3, ORANGE)
    # the worked sample
    nx = 0.6
    nz = math.sqrt(1 - nx * nx)
    px, py = cx + nx * R, cy - nz * R
    svg.line(cx, cy, px, py, GREEN, 2.0, arrow=True)
    svg.circle(px, py, 4.5, GREEN)
    svg.line(px, cy, px, py, GREEN, 1, dash="2 3")
    svg.line(cx, cy, px, cy, GREEN, 1, dash="2 3")
    svg.text(cx + nx * R / 2, cy + 16, "n_x = (x - cx) / r", 11, GREEN, "middle")
    svg.text(px + 8, (cy + py) / 2 + 4, "n_z", 11.5, GREEN)
    svg.text(px + 12, py - 8, "hit: depth = cz + n_z R", 11.5, GREEN)
    svg.text(cx - R - 14, cy + 40, "centre depth cz", 11, DIM)
    svg.circle(cx, cy, 3, FG)
    # captions
    lines = [
        "d = n_x^2 + n_y^2; covered if d <= 1",
        "n_z = sqrt(1 - d), the near root of the quadratic",
        "normal = (n_x, n_y, n_z), unit length: d + n_z^2 = 1",
        "depth = cz + n_z R (R world units, r pixels)",
    ]
    for i, line in enumerate(lines):
        svg.text(30, 385 + 18 * i, line, 12, FG)
    svg.text(24, 500, "The silhouette is r pixels across but the depth bulge is R world units, so the impostor is exactly a sphere only where", 11, DIM)
    svg.text(24, 516, "r = R * zoom * SCALE (Frame::world_sphere). Frame::sphere draws r = max(R * zoom, 3.5), about 18% smaller,", 11, DIM)
    svg.text(24, 532, "the size the views' spacing was tuned with. Lighting uses the normal in screen axes: the light does not turn with the camera.", 11, DIM)

    # ---------------- right: literal pixels ----------------
    svg.text(425, 82, "the literal pixels of a sphere, r = 20", 12.5, FG, weight="600")
    r = 20
    cell = 8
    ox, oy = 585, 262                    # centre pixel position
    base = (100, 160, 210)
    for j in range(-r, r + 1):
        for i in range(-r, r + 1):
            nx, ny = i / r, j / r
            d = nx * nx + ny * ny
            if d > 1.0:
                continue
            k = light(nx, ny)
            colour = tuple(max(0, min(255, int(v * k))) for v in base)
            svg.rect(ox + (i - 0.5) * cell, oy + (j - 0.5) * cell, cell, cell,
                     fill="#%02x%02x%02x" % colour)
    # terminator: great circle perpendicular to L, front half
    lnorm = math.sqrt(LX * LX + LY * LY + LZ * LZ)
    lvec = (-LX / lnorm, -LY / lnorm, LZ / lnorm)
    ref = (0.0, 0.0, 1.0) if abs(lvec[2]) < 0.9 else (1.0, 0.0, 0.0)
    u = (lvec[1] * ref[2] - lvec[2] * ref[1], lvec[2] * ref[0] - lvec[0] * ref[2], lvec[0] * ref[1] - lvec[1] * ref[0])
    un = math.sqrt(sum(c * c for c in u))
    u = tuple(c / un for c in u)
    v = (lvec[1] * u[2] - lvec[2] * u[1], lvec[2] * u[0] - lvec[0] * u[2], lvec[0] * u[1] - lvec[1] * u[0])
    full = []
    for i in range(0, 720):
        t = 2 * math.pi * i / 720
        full.append(tuple(u[k] * math.cos(t) + v[k] * math.sin(t) for k in range(3)))
    start = min(range(len(full)), key=lambda i: full[i][2])      # rotate so the front arc is contiguous
    full = full[start:] + full[:start]
    ring = [(ox + n[0] * r * cell, oy + n[1] * r * cell) for n in full if n[2] >= 0]
    svg.polyline(ring, YELLOW, 1.8, dash="5 3")
    # highlight and light arrow
    hx, hy = ox + lvec[0] * r * cell, oy + lvec[1] * r * cell
    svg.line(ox, oy, hx, hy, FG, 1.4, dash="2 3")
    svg.circle(hx, hy, 4.5, "none", FG, 1.6)
    svg.text(hx - 8, hy - 8, "peak 1.162 at n = L/|L|", 11, FG, "start")
    svg.circle(ox, oy, 2.5, FG)
    svg.text(ox + r * cell + 22, oy - 40, "light L = (-0.4, -0.5, 0.65)", 11.5, FG)
    svg.text(ox + r * cell + 22, oy - 24, "in (x right, y down, z to viewer);", 11, DIM)
    svg.text(ox + r * cell + 22, oy - 10, "|L| = 0.9124, not normalised", 11, DIM)
    svg.text(ox + r * cell + 22, oy + 18, "factor = 0.25 + max(n . L, 0)", 11.5, FG)
    svg.text(ox + r * cell + 22, oy + 34, "cap 1.2 is never reached:", 11, DIM)
    svg.text(ox + r * cell + 22, oy + 50, "0.25 + |L| = 1.162", 11, DIM)
    svg.text(ox + r * cell + 22, oy + 74, "yellow: terminator n . L = 0", 11, YELLOW)
    svg.text(ox + r * cell + 22, oy + 90, "14.4% of the disc is at 0.25", 11, YELLOW)
    # colour bar
    bx, by, bw, bh = 425, 445, 320, 12
    for i in range(110):
        k = AMBIENT + (1.162 - AMBIENT) * i / 109
        colour = tuple(max(0, min(255, int(c * k))) for c in base)
        svg.rect(bx + bw * i / 110, by, bw / 110 + 0.5, bh, fill="#%02x%02x%02x" % colour)
    svg.text(bx, by + 28, "0.25", 11, DIM)
    svg.text(bx + bw, by + 28, "1.162", 11, DIM, "end")
    svg.text(bx + bw / 2, by + 28, "light factor", 11, DIM, "middle")

    out = pathlib.Path("docs/media/figures/theory-sphere-impostor.svg")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(svg.render(arrows=(GREEN,)))


if __name__ == "__main__":
    main()
