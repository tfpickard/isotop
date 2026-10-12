"""A small, deterministic, standard-library SVG helper for the figures of the Cores, Cells, Strata,
Globe and Reef wiki pages.

Every figure it writes carries its own dark background rectangle (#0e1116) with light strokes and
text (#c9d1d9), so it reads in both GitHub themes. Nothing here reads the clock, the environment or
a random source: the same script always writes the same bytes.
"""

import math
import os

BACKGROUND = "#0e1116"
TEXT = "#c9d1d9"
MUTED = "#8b949e"
GRID = "#222a33"
AXIS = "#4b5563"
FONT = "system-ui, sans-serif"

# The colours isotop itself uses (src/palette.rs), so a figure and the view agree.
GOLD = "#ecb25c"      # SESSION and PERFORMANCE
TEAL = "#48b7c7"      # SYSTEM and EFFICIENCY
VIOLET = "#b288e8"    # CONTAINER
SLATE = "#687a9c"     # KERNEL
RED = "#f06054"       # the run queue (cores.rs QUEUE)
CYAN = "#6edcff"      # globe.rs DOWN
PINK = "#ff8cd2"      # globe.rs UP
GREEN = "#7bd88f"
SAND = "#d8c79a"


def num(value):
    """Short, stable decimal text for a coordinate."""
    text = "%.2f" % value
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return "0" if text in ("-0", "") else text


def esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


class Svg:
    """A figure canvas with a title, a description and a dark background."""

    def __init__(self, width, height, title, description=""):
        self.width = width
        self.height = height
        self.title = title
        self.description = description
        self.body = []

    def add(self, markup):
        self.body.append(markup)

    @staticmethod
    def _style(stroke, width, fill, opacity, dash):
        parts = ['fill="%s"' % fill]
        if stroke is not None:
            parts.append('stroke="%s" stroke-width="%s"' % (stroke, num(width)))
            parts.append('stroke-linejoin="round" stroke-linecap="round"')
        if opacity is not None:
            parts.append('opacity="%s"' % num(opacity))
        if dash:
            parts.append('stroke-dasharray="%s"' % dash)
        return " ".join(parts)

    def line(self, x1, y1, x2, y2, stroke=TEXT, width=1, dash=None, opacity=None):
        self.add('<line x1="%s" y1="%s" x2="%s" y2="%s" %s/>' % (
            num(x1), num(y1), num(x2), num(y2),
            self._style(stroke, width, "none", opacity, dash)))

    def rect(self, x, y, w, h, fill="none", stroke=None, width=1, rx=0, opacity=None, dash=None):
        self.add('<rect x="%s" y="%s" width="%s" height="%s" rx="%s" %s/>' % (
            num(x), num(y), num(w), num(h), num(rx),
            self._style(stroke, width, fill, opacity, dash)))

    def circle(self, cx, cy, r, fill="none", stroke=None, width=1, opacity=None, dash=None):
        self.add('<circle cx="%s" cy="%s" r="%s" %s/>' % (
            num(cx), num(cy), num(r), self._style(stroke, width, fill, opacity, dash)))

    def ellipse(self, cx, cy, rx, ry, fill="none", stroke=None, width=1, opacity=None, dash=None):
        self.add('<ellipse cx="%s" cy="%s" rx="%s" ry="%s" %s/>' % (
            num(cx), num(cy), num(rx), num(ry), self._style(stroke, width, fill, opacity, dash)))

    def polyline(self, points, stroke=TEXT, width=1, fill="none", opacity=None, dash=None):
        text = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        self.add('<polyline points="%s" %s/>' % (
            text, self._style(stroke, width, fill, opacity, dash)))

    def polygon(self, points, fill=TEXT, stroke=None, width=1, opacity=None, dash=None):
        text = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        self.add('<polygon points="%s" %s/>' % (
            text, self._style(stroke, width, fill, opacity, dash)))

    def path(self, d, stroke=TEXT, width=1, fill="none", opacity=None, dash=None):
        self.add('<path d="%s" %s/>' % (d, self._style(stroke, width, fill, opacity, dash)))

    def text(self, x, y, content, size=12, anchor="start", fill=TEXT, weight=None,
             rotate=None, italic=False, opacity=None):
        attrs = ['x="%s"' % num(x), 'y="%s"' % num(y), 'font-size="%s"' % num(size),
                 'text-anchor="%s"' % anchor, 'fill="%s"' % fill]
        if weight:
            attrs.append('font-weight="%s"' % weight)
        if italic:
            attrs.append('font-style="italic"')
        if opacity is not None:
            attrs.append('opacity="%s"' % num(opacity))
        if rotate is not None:
            attrs.append('transform="rotate(%s %s %s)"' % (num(rotate), num(x), num(y)))
        self.add("<text %s>%s</text>" % (" ".join(attrs), esc(content)))

    def arrow(self, x1, y1, x2, y2, stroke=TEXT, width=1.2, head=7, dash=None):
        self.line(x1, y1, x2, y2, stroke, width, dash)
        angle = math.atan2(y2 - y1, x2 - x1)
        left = (x2 - head * math.cos(angle - 0.4), y2 - head * math.sin(angle - 0.4))
        right = (x2 - head * math.cos(angle + 0.4), y2 - head * math.sin(angle + 0.4))
        self.polygon([(x2, y2), left, right], fill=stroke)

    def render(self):
        head = (
            '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %s %s" width="%s" height="%s" '
            'font-family="%s" role="img">\n' % (
                num(self.width), num(self.height), num(self.width), num(self.height), FONT))
        head += "<title>%s</title>\n" % esc(self.title)
        if self.description:
            head += "<desc>%s</desc>\n" % esc(self.description)
        head += '<rect width="%s" height="%s" fill="%s"/>\n' % (
            num(self.width), num(self.height), BACKGROUND)
        return head + "\n".join(self.body) + "\n</svg>\n"

    def save(self, slug):
        folder = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "media", "figures")
        os.makedirs(folder, exist_ok=True)
        path = os.path.normpath(os.path.join(folder, slug + ".svg"))
        with open(path, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(self.render())
        return path


class Axes:
    """A rectangle of the canvas with linear (or logarithmic) data coordinates."""

    def __init__(self, svg, left, top, width, height, xlim, ylim, xlog=False, ylog=False):
        self.svg = svg
        self.left, self.top, self.width, self.height = left, top, width, height
        self.xlim, self.ylim = xlim, ylim
        self.xlog, self.ylog = xlog, ylog

    def _scale(self, value, lim, log):
        low, high = lim
        if log:
            return (math.log(value) - math.log(low)) / (math.log(high) - math.log(low))
        return (value - low) / (high - low)

    def px(self, x):
        return self.left + self.width * self._scale(x, self.xlim, self.xlog)

    def py(self, y):
        return self.top + self.height * (1.0 - self._scale(y, self.ylim, self.ylog))

    def pt(self, x, y):
        return (self.px(x), self.py(y))

    def frame(self, xticks=(), yticks=(), xlabel="", ylabel="", xfmt=None, yfmt=None,
              grid=True, title=None):
        svg = self.svg
        xfmt = xfmt or (lambda v: ("%g" % v))
        yfmt = yfmt or (lambda v: ("%g" % v))
        for value in xticks:
            x = self.px(value)
            if grid:
                svg.line(x, self.top, x, self.top + self.height, GRID, 1)
            svg.line(x, self.top + self.height, x, self.top + self.height + 4, AXIS, 1)
            svg.text(x, self.top + self.height + 17, xfmt(value), 11, "middle", MUTED)
        for value in yticks:
            y = self.py(value)
            if grid:
                svg.line(self.left, y, self.left + self.width, y, GRID, 1)
            svg.line(self.left - 4, y, self.left, y, AXIS, 1)
            svg.text(self.left - 8, y + 4, yfmt(value), 11, "end", MUTED)
        svg.rect(self.left, self.top, self.width, self.height, "none", AXIS, 1)
        if xlabel:
            svg.text(self.left + self.width / 2, self.top + self.height + 36, xlabel, 12,
                     "middle", TEXT)
        if ylabel:
            svg.text(self.left - 44, self.top + self.height / 2, ylabel, 12, "middle", TEXT,
                     rotate=-90)
        if title:
            svg.text(self.left, self.top - 8, title, 13, "start", TEXT, weight="600")

    def curve(self, xs, ys, stroke=TEXT, width=1.6, dash=None, opacity=None):
        self.svg.polyline([self.pt(x, y) for x, y in zip(xs, ys)], stroke, width,
                          dash=dash, opacity=opacity)

    def steps(self, xs, ys, stroke=TEXT, width=1.6, opacity=None):
        """A right-continuous staircase: ys[i] holds from xs[i] to xs[i + 1]."""
        points = []
        for index, (x, y) in enumerate(zip(xs, ys)):
            points.append(self.pt(x, y))
            if index + 1 < len(xs):
                points.append(self.pt(xs[index + 1], y))
        self.svg.polyline(points, stroke, width, opacity=opacity)

    def area(self, xs, ys, fill, opacity=0.3, base=0.0):
        points = [self.pt(x, y) for x, y in zip(xs, ys)]
        points = [self.pt(xs[0], base)] + points + [self.pt(xs[-1], base)]
        self.svg.polygon(points, fill=fill, opacity=opacity)
