#!/usr/bin/env python3
"""Writes docs/media/figures/theory-frame-timeline.svg: the cadences of isotop's frame loop.

Top panel: four seconds of a default interactive session (--fps 10, --sample-ms 1000) with the
frame ticks, the samples taken on the frame thread, the background sampler's cycle and the event
thread. Bottom panel: the order of work inside one frame, bounded by the deadline
frame_start + 1/fps. The numbers (100 ms, 1000 ms, 2000 ms) are the defaults in src/cli.rs
(Options::fps, Options::sample_ms) and src/model.rs (BACKGROUND_PERIOD). Segment widths in the
bottom panel are schematic: the loop has no fixed split.

Standard library only; run from the repository root. Deterministic.
"""

import html
import pathlib

BG, FG, DIM, GRID = "#0e1116", "#c9d1d9", "#8b949e", "#30363d"
BLUE, ORANGE, GREEN, PURPLE, YELLOW, RED, TEAL = (
    "#58a6ff", "#f0883e", "#3fb950", "#bc8cff", "#d29922", "#f85149", "#39c5cf")
FONT = 'font-family="system-ui, sans-serif"'


class Svg:
    def __init__(self, width, height, title, description):
        self.width, self.height, self.title, self.description = width, height, title, description
        self.parts = []

    def text(self, x, y, s, size=12, fill=FG, anchor="start", weight="normal", style="normal"):
        self.parts.append(
            f'<text x="{x:.2f}" y="{y:.2f}" font-size="{size}" fill="{fill}" text-anchor="{anchor}" '
            f'font-weight="{weight}" font-style="{style}" {FONT}>{html.escape(s)}</text>')

    def line(self, x1, y1, x2, y2, stroke=FG, width=1.0, dash=None, opacity=1.0, marker=None):
        extra = f' stroke-dasharray="{dash}"' if dash else ""
        extra += f' marker-end="url(#{marker})"' if marker else ""
        self.parts.append(
            f'<line x1="{x1:.2f}" y1="{y1:.2f}" x2="{x2:.2f}" y2="{y2:.2f}" stroke="{stroke}" '
            f'stroke-width="{width}" stroke-opacity="{opacity}"{extra}/>')

    def rect(self, x, y, w, h, fill="none", stroke="none", width=1.0, opacity=1.0, rx=0):
        self.parts.append(
            f'<rect x="{x:.2f}" y="{y:.2f}" width="{w:.2f}" height="{h:.2f}" rx="{rx}" fill="{fill}" '
            f'fill-opacity="{opacity}" stroke="{stroke}" stroke-width="{width}"/>')

    def render(self):
        defs = (
            '<defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" '
            f'markerHeight="7" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="{FG}"/>'
            "</marker></defs>")
        body = "\n".join(self.parts)
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" '
            f'width="{self.width}" height="{self.height}" role="img">\n'
            f"<title>{html.escape(self.title)}</title>\n<desc>{html.escape(self.description)}</desc>\n"
            f'{defs}\n<rect width="{self.width}" height="{self.height}" fill="{BG}"/>\n{body}\n</svg>\n')


def main():
    svg = Svg(960, 560, "Frame loop cadences",
              "Frames every 100 ms, samples every 1000 ms, the background sampler every 2000 ms, "
              "and the order of work inside one frame up to its deadline.")
    svg.text(24, 30, "isotop frame loop: three clocks and one deadline", 17, FG, weight="600")
    svg.text(24, 50, "defaults: --fps 10, --sample-ms 1000, BACKGROUND_PERIOD 2 s", 12, DIM)

    # ---- top panel: four seconds ----
    left, right, top = 250, 920, 90
    span_ms = 4000.0

    def x_of(ms):
        return left + (right - left) * ms / span_ms

    lanes = [
        ("frame thread: frames", "1 / --fps", BLUE),
        ("frame thread: Collector::sample", "--sample-ms", ORANGE),
        ("isotop-sampler thread", "BACKGROUND_PERIOD", GREEN),
        ("isotop-events thread", "while LifeEvents demanded", PURPLE),
    ]
    lane_h = 44
    for i, (name, sub, colour) in enumerate(lanes):
        y = top + i * lane_h
        svg.rect(left, y, right - left, lane_h - 8, fill="#161b22", stroke=GRID)
        svg.text(left - 10, y + 16, name, 12, FG, "end")
        svg.text(left - 10, y + 30, sub, 10.5, DIM, "end")
    # axis
    axis_y = top + len(lanes) * lane_h + 2
    for ms in range(0, 4001, 500):
        x = x_of(ms)
        svg.line(x, top - 6, x, axis_y, GRID, 1, dash="2 4")
        svg.text(x, axis_y + 16, f"{ms / 1000:g} s", 11, DIM, "middle")
    # frames
    y = top
    for k in range(41):
        x = x_of(k * 100)
        svg.line(x, y + 6, x, y + lane_h - 14, BLUE, 1.5)
    # samples
    y = top + lane_h
    for k in range(5):
        x = x_of(k * 1000)
        svg.rect(x - 4, y + 6, 8, lane_h - 20, fill=ORANGE)
    svg.text(x_of(1000) + 8, y + 17, "record into the history ring", 10.5, DIM)
    # background cycles
    y = top + 2 * lane_h
    for k in range(3):
        x = x_of(k * 2000)
        svg.rect(x - 4, y + 6, 8, lane_h - 20, fill=GREEN)
    svg.text(x_of(0) + 10, y + 17, "sockets, files, locks, cgroups, NVML", 10.5, DIM)
    # events thread
    y = top + 3 * lane_h
    svg.rect(left + 1, y + 10, right - left - 2, lane_h - 28, fill=PURPLE, opacity=0.35, stroke=PURPLE)
    svg.text(left + 10, y + 22, "reads kernel events; fills a bounded LifeQueue that each sample drains", 10.5, FG)

    # ---- bottom panel: inside one frame ----
    y0 = 340
    svg.text(24, y0 - 18, "inside one frame (schematic widths; the loop is sequential)", 14, FG, weight="600")
    fx0, fx1 = 40, 920
    segments = [
        ("animate,\nsample\nif due", 0.09, ORANGE),
        ("camera\neasing,\ntour", 0.075, TEAL),
        ("Scene::\nrender\n(record)", 0.20, BLUE),
        ("rasterize:\nCPU or GPU\n+ readback", 0.22, RED),
        ("hover,\noverlay", 0.075, YELLOW),
        ("Terminal::\npresent", 0.13, PURPLE),
        ("event::poll\nuntil deadline\n(none if late)", 0.21, GRID),
    ]
    x = fx0
    total = fx1 - fx0
    for label, share, colour in segments:
        w = total * share
        last = colour == GRID
        svg.rect(x, y0, w, 62, fill=colour, opacity=0.30 if not last else 0.9, stroke=colour if not last else DIM)
        for j, row in enumerate(label.split("\n")):
            svg.text(x + w / 2, y0 + 20 + 14 * j, row, 11, FG, "middle")
        x += w
    svg.line(fx0, y0 + 80, fx1, y0 + 80, DIM, 1)
    svg.text(fx0, y0 + 98, "frame_start", 11, DIM)
    svg.text(fx1, y0 + 98, "deadline = frame_start + 1 / fps", 11, DIM, "end")
    svg.line(fx1, y0 - 6, fx1, y0 + 86, FG, 1.5, dash="4 3")

    notes = [
        "Every stage is on the one frame thread; only the slow sources and the event thread run beside it.",
        "At 10 fps the budget is 100 ms. A frame that overruns skips the wait and starts the next at once;",
        "the animation clock still advances by real time, and Stage::dt caps one step at MAX_DT = 0.5 s.",
    ]
    for i, line in enumerate(notes):
        svg.text(24, 470 + 18 * i, line, 12, DIM)
    svg.text(24, 535, "sources: src/interactive.rs, src/cli.rs, src/model.rs (BACKGROUND_PERIOD), "
                      "src/render.rs (MAX_DT)", 10.5, DIM, style="italic")

    out = pathlib.Path("docs/media/figures/theory-frame-timeline.svg")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(svg.render())


if __name__ == "__main__":
    main()
