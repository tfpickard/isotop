"""A small, deterministic, standard-library SVG plotting kit.

Shared by the figure scripts of the View-Switchboard, Planned-Plumbing, Planned-Morph,
Planned-Murmuration, Planned-Cortex, Planned-Ouija, Planned-Arbor, Planned-Canyon, Planned-Die,
Math-Reaction-Diffusion and Math-Spiking-Neurons wiki pages. Every figure carries its own dark
background (#0e1116) with light strokes and text (#c9d1d9) so it reads in both GitHub themes.
Nothing here touches the clock, the environment or a random source: the same script always
writes the same bytes. Raster panels are embedded as PNG data URIs built with zlib.
"""

import base64
import math
import os
import struct
import zlib

BACKGROUND = "#0e1116"
TEXT = "#c9d1d9"
MUTED = "#8b949e"
GRID = "#21262d"
AXIS = "#484f58"
FONT = "system-ui, sans-serif"
MONO = "ui-monospace, SFMono-Regular, Menlo, monospace"

BLUE = "#4c97f0"
ORANGE = "#e8743a"
AQUA = "#26b88a"
YELLOW = "#e0a020"
MAGENTA = "#df5f90"
VIOLET = "#9d92f0"
RED = "#ef7373"
SERIES = [BLUE, ORANGE, AQUA, YELLOW, MAGENTA, VIOLET, RED]


def num(value):
    """Short, stable decimal text for a coordinate."""
    text = "%.2f" % value
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return "0" if text in ("-0", "") else text


def escape(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


def repo_root():
    """The repository root, so a script writes docs/media/figures/ wherever it is run from."""
    here = os.path.dirname(os.path.abspath(__file__))
    return os.path.dirname(os.path.dirname(here))


def output_path(slug):
    return os.path.join(repo_root(), "docs", "media", "figures", slug + ".svg")


# ------------------------------------------------------------------------------------------
# PNG panels


def png_bytes(width, height, rows):
    """An RGB PNG. `rows` is a list of `height` bytes objects of length 3 * width."""
    raw = b"".join(b"\x00" + bytes(row) for row in rows)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def clamp8(value):
    return 0 if value < 0 else 255 if value > 255 else int(value + 0.5)


def gradient(stops, t):
    """Piecewise-linear colour at t in 0..1 from [(position, (r, g, b)), ...]."""
    if t <= stops[0][0]:
        return stops[0][1]
    for (a, ca), (b, cb) in zip(stops, stops[1:]):
        if t <= b:
            f = (t - a) / (b - a) if b > a else 0.0
            return tuple(ca[i] + (cb[i] - ca[i]) * f for i in range(3))
    return stops[-1][1]


# The ironbow stops of the die mockup (docs/mockups/code/die.py).
IRONBOW = [
    (0.00, (0, 0, 0)), (0.10, (10, 4, 60)), (0.24, (58, 4, 128)), (0.38, (128, 8, 148)),
    (0.52, (196, 30, 112)), (0.64, (232, 76, 40)), (0.76, (250, 140, 8)),
    (0.88, (255, 206, 40)), (0.96, (255, 246, 160)), (1.00, (255, 255, 250)),
]


# ------------------------------------------------------------------------------------------
# Canvas


class Svg:
    def __init__(self, width, height, title, description=""):
        self.width = width
        self.height = height
        self.title = title
        self.description = description
        self.body = []
        self.clips = 0

    def add(self, markup):
        self.body.append(markup)

    def text(self, x, y, content, size=12, anchor="start", fill=TEXT, weight=None,
             rotate=None, italic=False, opacity=None, family=None):
        attrs = ['x="%s"' % num(x), 'y="%s"' % num(y), 'font-size="%s"' % num(size),
                 'text-anchor="%s"' % anchor, 'fill="%s"' % fill]
        if family:
            attrs.append('font-family="%s"' % family)
        if weight:
            attrs.append('font-weight="%s"' % weight)
        if italic:
            attrs.append('font-style="italic"')
        if opacity is not None:
            attrs.append('opacity="%s"' % num(opacity))
        if rotate is not None:
            attrs.append('transform="rotate(%s %s %s)"' % (num(rotate), num(x), num(y)))
        self.add("<text %s>%s</text>" % (" ".join(attrs), escape(str(content))))

    def line(self, x1, y1, x2, y2, stroke=TEXT, width=1.0, dash=None, opacity=None, cap=None):
        attrs = ['x1="%s"' % num(x1), 'y1="%s"' % num(y1), 'x2="%s"' % num(x2),
                 'y2="%s"' % num(y2), 'stroke="%s"' % stroke, 'stroke-width="%s"' % num(width)]
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if opacity is not None:
            attrs.append('stroke-opacity="%s"' % num(opacity))
        if cap:
            attrs.append('stroke-linecap="%s"' % cap)
        self.add("<line %s/>" % " ".join(attrs))

    def rect(self, x, y, w, h, fill="none", stroke=None, width=1.0, opacity=None, rx=None):
        attrs = ['x="%s"' % num(x), 'y="%s"' % num(y), 'width="%s"' % num(w),
                 'height="%s"' % num(h), 'fill="%s"' % fill]
        if stroke:
            attrs += ['stroke="%s"' % stroke, 'stroke-width="%s"' % num(width)]
        if opacity is not None:
            attrs.append('fill-opacity="%s"' % num(opacity))
        if rx:
            attrs.append('rx="%s"' % num(rx))
        self.add("<rect %s/>" % " ".join(attrs))

    def circle(self, x, y, r, fill=TEXT, stroke=None, width=1.0, opacity=None):
        attrs = ['cx="%s"' % num(x), 'cy="%s"' % num(y), 'r="%s"' % num(r), 'fill="%s"' % fill]
        if stroke:
            attrs += ['stroke="%s"' % stroke, 'stroke-width="%s"' % num(width)]
        if opacity is not None:
            attrs.append('fill-opacity="%s"' % num(opacity))
        self.add("<circle %s/>" % " ".join(attrs))

    def polyline(self, points, stroke=TEXT, width=1.5, fill="none", dash=None, opacity=None,
                 close=False, join="round"):
        if not points:
            return
        data = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        tag = "polygon" if close else "polyline"
        attrs = ['points="%s"' % data, 'fill="%s"' % fill, 'stroke="%s"' % stroke,
                 'stroke-width="%s"' % num(width), 'stroke-linejoin="%s"' % join,
                 'stroke-linecap="round"']
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if opacity is not None:
            attrs.append('opacity="%s"' % num(opacity))
        self.add("<%s %s/>" % (tag, " ".join(attrs)))

    def polygon(self, points, fill=TEXT, stroke=None, width=1.0, opacity=None):
        data = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        attrs = ['points="%s"' % data, 'fill="%s"' % fill]
        if stroke:
            attrs += ['stroke="%s"' % stroke, 'stroke-width="%s"' % num(width)]
        if opacity is not None:
            attrs.append('fill-opacity="%s"' % num(opacity))
        self.add("<polygon %s/>" % " ".join(attrs))

    def path(self, d, stroke=TEXT, width=1.5, fill="none", dash=None, opacity=None):
        attrs = ['d="%s"' % d, 'fill="%s"' % fill, 'stroke="%s"' % stroke,
                 'stroke-width="%s"' % num(width), 'stroke-linecap="round"']
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if opacity is not None:
            attrs.append('opacity="%s"' % num(opacity))
        self.add("<path %s/>" % " ".join(attrs))

    def image(self, x, y, w, h, png, smooth=False):
        uri = "data:image/png;base64," + base64.b64encode(png).decode("ascii")
        style = "" if smooth else ' style="image-rendering:pixelated"'
        self.add('<image x="%s" y="%s" width="%s" height="%s" preserveAspectRatio="none"%s '
                 'href="%s"/>' % (num(x), num(y), num(w), num(h), style, uri))

    def clip(self, x, y, w, h):
        self.clips += 1
        name = "clip%d" % self.clips
        self.add('<clipPath id="%s"><rect x="%s" y="%s" width="%s" height="%s"/></clipPath>'
                 % (name, num(x), num(y), num(w), num(h)))
        return name

    def group(self, clip=None):
        self.add("<g%s>" % (' clip-path="url(#%s)"' % clip if clip else ""))

    def end(self):
        self.add("</g>")

    def title_text(self, content, y=22, size=15):
        self.text(self.width / 2, y, content, size=size, anchor="middle", weight="600")

    def render(self):
        head = (
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %s %s" width="%s" '
            'height="%s" font-family="%s" role="img">\n' % (
                num(self.width), num(self.height), num(self.width), num(self.height), FONT)
            + "<title>%s</title>\n" % escape(self.title)
            + ("<desc>%s</desc>\n" % escape(self.description) if self.description else "")
            + '<rect width="100%" height="100%" fill="' + BACKGROUND + '"/>\n'
        )
        return head + "\n".join(self.body) + "\n</svg>\n"

    def save(self, slug):
        path = output_path(slug)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(self.render())
        return path


# ------------------------------------------------------------------------------------------
# Axes


def nice_step(span, target):
    raw = span / max(target, 1)
    magnitude = 10 ** math.floor(math.log10(raw))
    for factor in (1, 2, 2.5, 5, 10):
        if raw <= factor * magnitude:
            return factor * magnitude
    return 10 * magnitude


def linear_ticks(lo, hi, target=6):
    step = nice_step(hi - lo, target)
    first = math.ceil(lo / step - 1e-9) * step
    ticks = []
    value = first
    while value <= hi + step * 1e-9:
        ticks.append(0.0 if abs(value) < step * 1e-9 else value)
        value += step
    return ticks


def log_ticks(lo, hi):
    ticks = []
    exponent = math.floor(math.log10(lo))
    while 10 ** exponent <= hi * 1.0000001:
        if 10 ** exponent >= lo * 0.9999999:
            ticks.append(10 ** exponent)
        exponent += 1
    return ticks


def tick_label(value, log=False):
    if log:
        exponent = round(math.log10(value))
        if -3 <= exponent <= 4:
            return ("%g" % value)
        return "1e%d" % exponent
    text = "%.6g" % value
    return text


class Axes:
    """A plotting area at (x, y) of size (w, h) pixels with data limits xlim and ylim."""

    def __init__(self, svg, x, y, w, h, xlim, ylim, xlabel="", ylabel="", title="",
                 xlog=False, ylog=False, xticks=None, yticks=None, xtick_labels=None,
                 ytick_labels=None, grid=True, tick_size=11, label_size=12):
        self.svg = svg
        self.x, self.y, self.w, self.h = x, y, w, h
        self.xlim, self.ylim = xlim, ylim
        self.xlog, self.ylog = xlog, ylog
        self.clip_name = svg.clip(x, y, w, h)
        if grid:
            self._ticks(xticks, yticks, xtick_labels, ytick_labels, tick_size)
        svg.rect(x, y, w, h, fill="none", stroke=AXIS, width=1)
        if xlabel:
            svg.text(x + w / 2, y + h + 38, xlabel, size=label_size, anchor="middle", fill=TEXT)
        if ylabel:
            svg.text(x - 48, y + h / 2, ylabel, size=label_size, anchor="middle", rotate=-90)
        if title:
            svg.text(x + w / 2, y - 8, title, size=13, anchor="middle", weight="600")

    # data -> pixels
    def px(self, value):
        lo, hi = self.xlim
        if self.xlog:
            value, lo, hi = math.log10(value), math.log10(lo), math.log10(hi)
        return self.x + (value - lo) / (hi - lo) * self.w

    def py(self, value):
        lo, hi = self.ylim
        if self.ylog:
            value, lo, hi = math.log10(value), math.log10(lo), math.log10(hi)
        return self.y + self.h - (value - lo) / (hi - lo) * self.h

    def _ticks(self, xticks, yticks, xlabels, ylabels, size):
        svg = self.svg
        xs = xticks if xticks is not None else (
            log_ticks(*self.xlim) if self.xlog else linear_ticks(*self.xlim))
        ys = yticks if yticks is not None else (
            log_ticks(*self.ylim) if self.ylog else linear_ticks(*self.ylim, target=5))
        for i, value in enumerate(xs):
            px = self.px(value)
            svg.line(px, self.y, px, self.y + self.h, stroke=GRID, width=1)
            label = xlabels[i] if xlabels else tick_label(value, self.xlog)
            svg.text(px, self.y + self.h + 16, label, size=size, anchor="middle", fill=MUTED)
        for i, value in enumerate(ys):
            py = self.py(value)
            svg.line(self.x, py, self.x + self.w, py, stroke=GRID, width=1)
            label = ylabels[i] if ylabels else tick_label(value, self.ylog)
            svg.text(self.x - 7, py + 4, label, size=size, anchor="end", fill=MUTED)

    # drawing
    def _pts(self, xs, ys):
        return [(self.px(x), self.py(y)) for x, y in zip(xs, ys)
                if not (self.xlog and x <= 0) and not (self.ylog and y <= 0)
                and y == y and x == x]

    def line(self, xs, ys, color=BLUE, width=1.8, dash=None, opacity=None):
        self.svg.group(self.clip_name)
        self.svg.polyline(self._pts(xs, ys), stroke=color, width=width, dash=dash, opacity=opacity)
        self.svg.end()

    def step(self, xs, ys, color=BLUE, width=1.8):
        points = []
        for i, (x, y) in enumerate(zip(xs, ys)):
            if i:
                points.append((self.px(x), self.py(ys[i - 1])))
            points.append((self.px(x), self.py(y)))
        self.svg.group(self.clip_name)
        self.svg.polyline(points, stroke=color, width=width)
        self.svg.end()

    def points(self, xs, ys, color=BLUE, r=3.0, opacity=None, stroke=None):
        self.svg.group(self.clip_name)
        for px, py in self._pts(xs, ys):
            self.svg.circle(px, py, r, fill=color, opacity=opacity, stroke=stroke)
        self.svg.end()

    def fill_between(self, xs, lows, highs, color=BLUE, opacity=0.25):
        upper = [(self.px(x), self.py(y)) for x, y in zip(xs, highs)]
        lower = [(self.px(x), self.py(y)) for x, y in zip(reversed(xs), reversed(lows))]
        self.svg.group(self.clip_name)
        self.svg.polygon(upper + lower, fill=color, opacity=opacity)
        self.svg.end()

    def band(self, x0, x1, color=YELLOW, opacity=0.12):
        left, right = self.px(x0), self.px(x1)
        self.svg.group(self.clip_name)
        self.svg.rect(min(left, right), self.y, abs(right - left), self.h, fill=color,
                      opacity=opacity)
        self.svg.end()

    def hband(self, y0, y1, color=YELLOW, opacity=0.12):
        top, bottom = self.py(y1), self.py(y0)
        self.svg.group(self.clip_name)
        self.svg.rect(self.x, min(top, bottom), self.w, abs(bottom - top), fill=color,
                      opacity=opacity)
        self.svg.end()

    def hline(self, value, color=MUTED, width=1.2, dash="5 4"):
        py = self.py(value)
        self.svg.line(self.x, py, self.x + self.w, py, stroke=color, width=width, dash=dash)

    def vline(self, value, color=MUTED, width=1.2, dash="5 4"):
        px = self.px(value)
        self.svg.line(px, self.y, px, self.y + self.h, stroke=color, width=width, dash=dash)

    def label(self, x, y, content, color=TEXT, size=11, anchor="start", dx=0, dy=0, weight=None):
        self.svg.text(self.px(x) + dx, self.py(y) + dy, content, size=size, anchor=anchor,
                      fill=color, weight=weight)

    def legend(self, entries, corner="tr", size=11, pad=8, line_height=17):
        """entries: [(label, color, dash_or_None)]; drawn inside the axes."""
        width = max(len(label) for label, _, _ in entries) * size * 0.58 + 34
        height = len(entries) * line_height + 8
        x0 = self.x + pad if "l" in corner else self.x + self.w - width - pad
        y0 = self.y + pad if "t" in corner else self.y + self.h - height - pad
        self.svg.rect(x0, y0, width, height, fill=BACKGROUND, stroke=AXIS, width=1, opacity=0.88)
        for i, (label, color, dash) in enumerate(entries):
            cy = y0 + 14 + i * line_height
            self.svg.line(x0 + 7, cy - 4, x0 + 25, cy - 4, stroke=color, width=2.2, dash=dash)
            self.svg.text(x0 + 31, cy, label, size=size, fill=TEXT)


def colorbar(svg, x, y, w, h, stops, lo, hi, label="", ticks=None, vertical=True):
    """A gradient bar drawn as a tiny PNG, with tick labels."""
    n = 256
    if vertical:
        rows = []
        for j in range(n):
            t = 1 - j / (n - 1)
            colour = gradient(stops, t)
            rows.append(bytes(clamp8(c) for c in colour) * 2)
        svg.image(x, y, w, h, png_bytes(2, n, rows), smooth=True)
    svg.rect(x, y, w, h, fill="none", stroke=AXIS, width=1)
    for value in (ticks if ticks is not None else linear_ticks(lo, hi, 5)):
        t = (value - lo) / (hi - lo)
        py = y + h - t * h
        svg.line(x + w, py, x + w + 4, py, stroke=MUTED, width=1)
        svg.text(x + w + 7, py + 4, "%g" % value, size=10, fill=MUTED)
    if label:
        svg.text(x + w / 2, y - 8, label, size=11, anchor="middle", fill=TEXT)


def heat_png(grid, stops, lo, hi):
    """A PNG from a list of rows of floats, mapped through a gradient between lo and hi."""
    height, width = len(grid), len(grid[0])
    rows = []
    span = hi - lo if hi > lo else 1.0
    cache = {}
    for row in grid:
        out = bytearray()
        for value in row:
            t = (value - lo) / span
            t = 0.0 if t < 0 else 1.0 if t > 1 else t
            key = int(t * 1023)
            colour = cache.get(key)
            if colour is None:
                colour = bytes(clamp8(c) for c in gradient(stops, key / 1023))
                cache[key] = colour
            out += colour
        rows.append(bytes(out))
    return png_bytes(width, height, rows)


def rgb_png(pixels):
    """A PNG from a list of rows of (r, g, b) tuples."""
    height, width = len(pixels), len(pixels[0])
    rows = [bytes(clamp8(c) for pixel in row for c in pixel) for row in pixels]
    return png_bytes(width, height, rows)
