"""A tiny, deterministic, standard-library SVG plotting helper.

Shared by the figure scripts of the Planned-* wiki pages (mold, chamber, horizon, hr, hive,
anthill) and Math-Schwarzschild. Every figure it writes has its own dark background rectangle
(#0e1116) with light strokes and text (#c9d1d9), so it reads in both GitHub themes. Nothing here
touches the clock, the environment or a random source: the same script always writes the same
bytes.
"""

import math
import os

BACKGROUND = "#0e1116"
TEXT = "#c9d1d9"
MUTED = "#8b949e"
GRID = "#21262d"
AXIS = "#484f58"
FONT = "system-ui, sans-serif"

# Categorical slots in a fixed order, the dark-surface steps of a validated palette.
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


class Svg:
    """A figure canvas with a title, a description and a dark background."""

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
             rotate=None, italic=False, opacity=None, baseline=None):
        attrs = ['x="%s"' % num(x), 'y="%s"' % num(y), 'font-size="%s"' % num(size),
                 'text-anchor="%s"' % anchor, 'fill="%s"' % fill]
        if weight:
            attrs.append('font-weight="%s"' % weight)
        if italic:
            attrs.append('font-style="italic"')
        if opacity is not None:
            attrs.append('opacity="%s"' % num(opacity))
        if baseline:
            attrs.append('dominant-baseline="%s"' % baseline)
        if rotate is not None:
            attrs.append('transform="rotate(%s %s %s)"' % (num(rotate), num(x), num(y)))
        self.add("<text %s>%s</text>" % (" ".join(attrs), escape(content)))

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

    def polyline(self, points, stroke=TEXT, width=1.5, dash=None, opacity=None, fill="none",
                 clip=None, close=False, fill_opacity=None, join="round"):
        if len(points) < 2:
            return
        data = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        tag = "polygon" if close else "polyline"
        attrs = ['points="%s"' % data, 'fill="%s"' % fill, 'stroke="%s"' % stroke,
                 'stroke-width="%s"' % num(width), 'stroke-linejoin="%s"' % join,
                 'stroke-linecap="round"']
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if opacity is not None:
            attrs.append('stroke-opacity="%s"' % num(opacity))
        if fill_opacity is not None:
            attrs.append('fill-opacity="%s"' % num(fill_opacity))
        if clip:
            attrs.append('clip-path="url(#%s)"' % clip)
        self.add("<%s %s/>" % (tag, " ".join(attrs)))

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

    def circle(self, x, y, r, fill="none", stroke=None, width=1.0, opacity=None,
               stroke_opacity=None, dash=None, clip=None):
        attrs = ['cx="%s"' % num(x), 'cy="%s"' % num(y), 'r="%s"' % num(r), 'fill="%s"' % fill]
        if stroke:
            attrs += ['stroke="%s"' % stroke, 'stroke-width="%s"' % num(width)]
        if opacity is not None:
            attrs.append('fill-opacity="%s"' % num(opacity))
        if stroke_opacity is not None:
            attrs.append('stroke-opacity="%s"' % num(stroke_opacity))
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if clip:
            attrs.append('clip-path="url(#%s)"' % clip)
        self.add("<circle %s/>" % " ".join(attrs))

    def clip_rect(self, x, y, w, h):
        """Declares a clip path and returns its id."""
        self.clips += 1
        name = "clip%d" % self.clips
        self.add('<defs><clipPath id="%s"><rect x="%s" y="%s" width="%s" height="%s"/></clipPath>'
                 "</defs>" % (name, num(x), num(y), num(w), num(h)))
        return name

    def legend(self, x, y, entries, size=11, gap=17):
        """entries: (label, colour, style) with style 'line', 'dash' or 'dot'."""
        for index, (label, colour, style) in enumerate(entries):
            row = y + index * gap
            if style == "dot":
                self.circle(x + 8, row - 4, 3.2, fill=colour)
            else:
                self.line(x, row - 4, x + 16, row - 4, stroke=colour, width=2.0,
                          dash="4 3" if style == "dash" else None)
            self.text(x + 22, row, label, size=size)

    def render(self):
        head = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" '
                'height="%d" font-family="%s">' % (self.width, self.height, self.width,
                                                   self.height, FONT))
        parts = ['<?xml version="1.0" encoding="UTF-8"?>', head,
                 "<title>%s</title>" % escape(self.title)]
        if self.description:
            parts.append("<desc>%s</desc>" % escape(self.description))
        parts.append('<rect width="%d" height="%d" fill="%s"/>' % (self.width, self.height,
                                                                   BACKGROUND))
        parts.extend(self.body)
        parts.append("</svg>")
        return "\n".join(parts) + "\n"

    def save(self, path):
        folder = os.path.dirname(path)
        if folder:
            os.makedirs(folder, exist_ok=True)
        with open(path, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(self.render())


def nice_ticks(low, high, target=6):
    """Round tick positions covering [low, high]."""
    span = high - low
    if span <= 0:
        return [low]
    raw = span / max(target, 1)
    magnitude = 10 ** math.floor(math.log10(raw))
    for factor in (1, 2, 2.5, 5, 10):
        step = factor * magnitude
        if span / step <= target + 1:
            break
    first = math.ceil(low / step - 1e-9) * step
    ticks = []
    value = first
    while value <= high + 1e-9 * span:
        ticks.append(round(value, 12))
        value += step
    return ticks


def tick_text(value):
    if abs(value) >= 1e5 or (0 < abs(value) < 1e-3):
        return ("%.0e" % value).replace("e+0", "e").replace("e-0", "e-").replace("e+", "e")
    text = "%.4f" % value
    text = text.rstrip("0").rstrip(".")
    return "0" if text in ("-0", "") else text


class Axes:
    """A plotting rectangle with linear or logarithmic axes, ticks, grid and labels."""

    def __init__(self, svg, left, top, width, height, xlim, ylim, xlog=False, ylog=False,
                 xlabel=None, ylabel=None, title=None, xticks=None, yticks=None,
                 xtick_labels=None, ytick_labels=None, grid=True, invert_x=False,
                 invert_y=False, frame=True, tick_size=10.5, label_size=12, title_size=13,
                 show_x=True, show_y=True, equal=False):
        self.svg = svg
        self.left, self.top, self.width, self.height = left, top, width, height
        self.xlim, self.ylim = xlim, ylim
        self.xlog, self.ylog = xlog, ylog
        self.invert_x, self.invert_y = invert_x, invert_y
        self.clip = svg.clip_rect(left, top, width, height)
        if grid:
            self._grid(xticks, yticks)
        if frame:
            svg.rect(left, top, width, height, stroke=AXIS, width=1.0)
        self._ticks(xticks, yticks, xtick_labels, ytick_labels, tick_size, show_x, show_y)
        if xlabel:
            svg.text(left + width / 2, top + height + 38, xlabel, size=label_size,
                     anchor="middle", fill=TEXT)
        if ylabel:
            svg.text(left - 46, top + height / 2, ylabel, size=label_size, anchor="middle",
                     rotate=-90)
        if title:
            svg.text(left, top - 9, title, size=title_size, weight="600")

    def _scale(self, value, limits, log):
        low, high = limits
        if log:
            return (math.log(value) - math.log(low)) / (math.log(high) - math.log(low))
        return (value - low) / (high - low)

    def px(self, x):
        fraction = self._scale(x, self.xlim, self.xlog)
        if self.invert_x:
            fraction = 1.0 - fraction
        return self.left + fraction * self.width

    def py(self, y):
        fraction = self._scale(y, self.ylim, self.ylog)
        if not self.invert_y:
            fraction = 1.0 - fraction
        return self.top + fraction * self.height

    def _tick_values(self, ticks, limits, log):
        if ticks is not None:
            return list(ticks)
        if log:
            first = math.ceil(math.log10(limits[0]) - 1e-9)
            last = math.floor(math.log10(limits[1]) + 1e-9)
            return [10.0 ** e for e in range(first, last + 1)]
        return nice_ticks(limits[0], limits[1])

    def _grid(self, xticks, yticks):
        for value in self._tick_values(xticks, self.xlim, self.xlog):
            x = self.px(value)
            self.svg.line(x, self.top, x, self.top + self.height, stroke=GRID, width=1)
        for value in self._tick_values(yticks, self.ylim, self.ylog):
            y = self.py(value)
            self.svg.line(self.left, y, self.left + self.width, y, stroke=GRID, width=1)

    def _ticks(self, xticks, yticks, xlabels, ylabels, size, show_x, show_y):
        xs = self._tick_values(xticks, self.xlim, self.xlog)
        ys = self._tick_values(yticks, self.ylim, self.ylog)
        if show_x:
            for index, value in enumerate(xs):
                x = self.px(value)
                self.svg.line(x, self.top + self.height, x, self.top + self.height + 4,
                              stroke=AXIS, width=1)
                label = xlabels[index] if xlabels else tick_text(value)
                self.svg.text(x, self.top + self.height + 17, label, size=size,
                              anchor="middle", fill=MUTED)
        if show_y:
            for index, value in enumerate(ys):
                y = self.py(value)
                self.svg.line(self.left - 4, y, self.left, y, stroke=AXIS, width=1)
                label = ylabels[index] if ylabels else tick_text(value)
                self.svg.text(self.left - 8, y + 3.5, label, size=size, anchor="end",
                              fill=MUTED)

    def plot(self, xs, ys, stroke=BLUE, width=1.8, dash=None, opacity=None):
        points = [(self.px(x), self.py(y)) for x, y in zip(xs, ys)]
        self.svg.polyline(points, stroke=stroke, width=width, dash=dash, opacity=opacity,
                          clip=self.clip)

    def fill_between(self, xs, lows, highs, fill=BLUE, opacity=0.2):
        upper = [(self.px(x), self.py(y)) for x, y in zip(xs, highs)]
        lower = [(self.px(x), self.py(y)) for x, y in zip(xs, lows)]
        self.svg.polyline(upper + lower[::-1], stroke="none", width=0, fill=fill,
                          fill_opacity=opacity, close=True, clip=self.clip)

    def dots(self, xs, ys, fill=BLUE, radius=3.0, opacity=None, stroke=None):
        for x, y in zip(xs, ys):
            self.svg.circle(self.px(x), self.py(y), radius, fill=fill, opacity=opacity,
                            stroke=stroke, clip=self.clip)

    def hline(self, y, stroke=MUTED, width=1.0, dash="4 3", opacity=0.9):
        py = self.py(y)
        self.svg.line(self.left, py, self.left + self.width, py, stroke=stroke, width=width,
                      dash=dash, opacity=opacity)

    def vline(self, x, stroke=MUTED, width=1.0, dash="4 3", opacity=0.9):
        px = self.px(x)
        self.svg.line(px, self.top, px, self.top + self.height, stroke=stroke, width=width,
                      dash=dash, opacity=opacity)

    def label(self, x, y, content, size=11, anchor="start", fill=TEXT, dx=0, dy=0, **kwargs):
        self.svg.text(self.px(x) + dx, self.py(y) + dy, content, size=size, anchor=anchor,
                      fill=fill, **kwargs)
