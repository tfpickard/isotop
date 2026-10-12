"""A small, deterministic, standard-library SVG plotting kit for the figures of the Coop, Matrix,
Ptolemy, Tesseract and Necropolis wiki pages and their Math pages (Fourier epicycles, flocking,
stellar photometry, Lorentz and Schlegel).

Every figure it writes has its own dark background rectangle (#0e1116) with light strokes and
text (#c9d1d9), so it reads in both GitHub themes. Nothing here touches the clock, the
environment or a random source: the same script always writes the same bytes.
"""

import math
import os

BACKGROUND = "#0e1116"
TEXT = "#c9d1d9"
MUTED = "#8b949e"
GRID = "#21262d"
AXIS = "#484f58"
FONT = "system-ui, sans-serif"

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


def nice_ticks(lo, hi, target=6):
    """Round tick values covering [lo, hi]."""
    span = hi - lo
    if span <= 0:
        return [lo]
    raw = span / max(target, 1)
    magnitude = 10 ** math.floor(math.log10(raw))
    for factor in (1, 2, 2.5, 5, 10):
        step = factor * magnitude
        if span / step <= target + 1:
            break
    first = math.ceil(lo / step - 1e-9) * step
    ticks = []
    value = first
    while value <= hi + step * 1e-9:
        ticks.append(round(value, 10))
        value += step
    return ticks


def tick_label(value):
    if abs(value) >= 1e5 or (0 < abs(value) < 1e-3):
        return "%.0e" % value
    text = "%.4g" % value
    return text


class Svg:
    """A canvas with a title, a description and a dark background."""

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
             rotate=None, italic=False, opacity=None, mono=False):
        attrs = ['x="%s"' % num(x), 'y="%s"' % num(y), 'font-size="%s"' % num(size),
                 'text-anchor="%s"' % anchor, 'fill="%s"' % fill]
        if weight:
            attrs.append('font-weight="%s"' % weight)
        if italic:
            attrs.append('font-style="italic"')
        if mono:
            attrs.append('font-family="ui-monospace, monospace"')
        if opacity is not None:
            attrs.append('opacity="%s"' % num(opacity))
        if rotate is not None:
            attrs.append('transform="rotate(%s %s %s)"' % (num(rotate), num(x), num(y)))
        self.add("<text %s>%s</text>" % (" ".join(attrs), escape(content)))

    def line(self, x1, y1, x2, y2, stroke=TEXT, width=1.0, dash=None, opacity=None):
        attrs = ['x1="%s"' % num(x1), 'y1="%s"' % num(y1), 'x2="%s"' % num(x2),
                 'y2="%s"' % num(y2), 'stroke="%s"' % stroke, 'stroke-width="%s"' % num(width)]
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if opacity is not None:
            attrs.append('stroke-opacity="%s"' % num(opacity))
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

    def circle(self, x, y, r, fill="none", stroke=None, width=1.0, opacity=None, dash=None):
        attrs = ['cx="%s"' % num(x), 'cy="%s"' % num(y), 'r="%s"' % num(r), 'fill="%s"' % fill]
        if stroke:
            attrs += ['stroke="%s"' % stroke, 'stroke-width="%s"' % num(width)]
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if opacity is not None:
            attrs.append('opacity="%s"' % num(opacity))
        self.add("<circle %s/>" % " ".join(attrs))

    def path(self, points, stroke=TEXT, width=1.5, fill="none", close=False, dash=None,
             opacity=None, fill_opacity=None, join="round"):
        if not points:
            return
        data = "M" + " L".join("%s %s" % (num(x), num(y)) for x, y in points)
        if close:
            data += " Z"
        attrs = ['d="%s"' % data, 'fill="%s"' % fill, 'stroke="%s"' % stroke,
                 'stroke-width="%s"' % num(width), 'stroke-linejoin="%s"' % join,
                 'stroke-linecap="round"']
        if dash:
            attrs.append('stroke-dasharray="%s"' % dash)
        if opacity is not None:
            attrs.append('stroke-opacity="%s"' % num(opacity))
        if fill_opacity is not None:
            attrs.append('fill-opacity="%s"' % num(fill_opacity))
        self.add("<path %s/>" % " ".join(attrs))

    def clip_begin(self, x, y, w, h):
        self.clips += 1
        name = "clip%d" % self.clips
        self.add('<clipPath id="%s"><rect x="%s" y="%s" width="%s" height="%s"/></clipPath>'
                 % (name, num(x), num(y), num(w), num(h)))
        self.add('<g clip-path="url(#%s)">' % name)

    def clip_end(self):
        self.add("</g>")

    def render(self):
        head = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" '
                'height="%d" font-family="%s">' % (self.width, self.height, self.width,
                                                    self.height, FONT))
        parts = [head, "<title>%s</title>" % escape(self.title)]
        if self.description:
            parts.append("<desc>%s</desc>" % escape(self.description))
        parts.append('<rect width="100%%" height="100%%" fill="%s"/>' % BACKGROUND)
        parts.extend(self.body)
        parts.append("</svg>")
        return "\n".join(parts) + "\n"

    def save(self, name):
        """Writes docs/media/figures/<name>.svg relative to the repository root."""
        folder = os.path.join("docs", "media", "figures")
        os.makedirs(folder, exist_ok=True)
        path = os.path.join(folder, name + ".svg")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(self.render())
        return path


class Axes:
    """A plotting rectangle with a linear or logarithmic mapping and its own ticks."""

    def __init__(self, svg, x, y, w, h, xlim, ylim, xlabel="", ylabel="", title="",
                 xlog=False, ylog=False, xticks=None, yticks=None, xlabels=None, ylabels=None,
                 grid=True, frame=True, title_size=13, label_size=11, tick_size=10,
                 equal=False):
        self.svg = svg
        self.x, self.y, self.w, self.h = x, y, w, h
        self.xlim, self.ylim = xlim, ylim
        self.xlog, self.ylog = xlog, ylog
        self.equal = equal
        if equal:
            # shrink the longer side so one data unit has the same length on both axes
            sx = w / (xlim[1] - xlim[0])
            sy = h / (ylim[1] - ylim[0])
            s = min(sx, sy)
            nw, nh = s * (xlim[1] - xlim[0]), s * (ylim[1] - ylim[0])
            self.x += (w - nw) / 2
            self.y += (h - nh) / 2
            self.w, self.h = nw, nh
        if grid or frame or xticks is not None:
            self._decorate(xlabel, ylabel, title, xticks, yticks, xlabels, ylabels, grid, frame,
                           title_size, label_size, tick_size)

    def px(self, value):
        lo, hi = self.xlim
        if self.xlog:
            t = (math.log10(value) - math.log10(lo)) / (math.log10(hi) - math.log10(lo))
        else:
            t = (value - lo) / (hi - lo)
        return self.x + t * self.w

    def py(self, value):
        lo, hi = self.ylim
        if self.ylog:
            t = (math.log10(value) - math.log10(lo)) / (math.log10(hi) - math.log10(lo))
        else:
            t = (value - lo) / (hi - lo)
        return self.y + self.h - t * self.h

    def pt(self, xv, yv):
        return (self.px(xv), self.py(yv))

    def _log_ticks(self, lim):
        lo, hi = math.floor(math.log10(lim[0])), math.ceil(math.log10(lim[1]))
        return [10.0 ** e for e in range(lo, hi + 1) if lim[0] * (1 - 1e-9) <= 10.0 ** e <= lim[1] * (1 + 1e-9)]

    def _decorate(self, xlabel, ylabel, title, xticks, yticks, xlabels, ylabels, grid, frame,
                  title_size, label_size, tick_size):
        svg = self.svg
        if xticks is None:
            xticks = self._log_ticks(self.xlim) if self.xlog else nice_ticks(*self.xlim)
        if yticks is None:
            yticks = self._log_ticks(self.ylim) if self.ylog else nice_ticks(*self.ylim)
        for i, value in enumerate(xticks):
            if not (self.xlim[0] - 1e-9 <= value <= self.xlim[1] + 1e-9):
                continue
            px = self.px(value)
            if grid:
                svg.line(px, self.y, px, self.y + self.h, GRID, 1)
            label = xlabels[i] if xlabels else tick_label(value)
            svg.line(px, self.y + self.h, px, self.y + self.h + 4, AXIS, 1)
            svg.text(px, self.y + self.h + 16, label, tick_size, "middle", MUTED)
        for i, value in enumerate(yticks):
            if not (self.ylim[0] - 1e-9 <= value <= self.ylim[1] + 1e-9):
                continue
            py = self.py(value)
            if grid:
                svg.line(self.x, py, self.x + self.w, py, GRID, 1)
            label = ylabels[i] if ylabels else tick_label(value)
            svg.line(self.x - 4, py, self.x, py, AXIS, 1)
            svg.text(self.x - 7, py + 3.5, label, tick_size, "end", MUTED)
        if frame:
            svg.rect(self.x, self.y, self.w, self.h, "none", AXIS, 1)
        if xlabel:
            svg.text(self.x + self.w / 2, self.y + self.h + 34, xlabel, label_size, "middle", TEXT)
        if ylabel:
            svg.text(self.x - 44, self.y + self.h / 2, ylabel, label_size, "middle", TEXT,
                     rotate=-90)
        if title:
            svg.text(self.x, self.y - 9, title, title_size, "start", TEXT, weight="600")

    def line(self, xs, ys, color=BLUE, width=1.8, dash=None, opacity=None):
        self.svg.clip_begin(self.x, self.y, self.w, self.h)
        self.svg.path([self.pt(a, b) for a, b in zip(xs, ys)], color, width, dash=dash,
                      opacity=opacity)
        self.svg.clip_end()

    def polyline(self, points, color=BLUE, width=1.8, dash=None, opacity=None, close=False,
                 fill="none", fill_opacity=None):
        self.svg.clip_begin(self.x, self.y, self.w, self.h)
        self.svg.path([self.pt(a, b) for a, b in points], color, width, fill=fill, close=close,
                      dash=dash, opacity=opacity, fill_opacity=fill_opacity)
        self.svg.clip_end()

    def points(self, xs, ys, color=BLUE, radius=2.5, opacity=None):
        self.svg.clip_begin(self.x, self.y, self.w, self.h)
        for a, b in zip(xs, ys):
            self.svg.circle(self.px(a), self.py(b), radius, color, opacity=opacity)
        self.svg.clip_end()

    def vline(self, value, color=MUTED, width=1.0, dash="4 3"):
        px = self.px(value)
        self.svg.line(px, self.y, px, self.y + self.h, color, width, dash)

    def hline(self, value, color=MUTED, width=1.0, dash="4 3"):
        py = self.py(value)
        self.svg.line(self.x, py, self.x + self.w, py, color, width, dash)

    def label(self, xv, yv, content, size=10.5, anchor="start", fill=TEXT, dx=0, dy=0, **kw):
        self.svg.text(self.px(xv) + dx, self.py(yv) + dy, content, size, anchor, fill, **kw)

    def legend(self, entries, x=None, y=None, size=11):
        """entries: list of (label, color, kind) with kind 'line', 'dash' or 'dot'."""
        x = self.x + 10 if x is None else x
        y = self.y + 14 if y is None else y
        for i, (label, color, kind) in enumerate(entries):
            yy = y + i * (size + 6)
            if kind == "dot":
                self.svg.circle(x + 8, yy - 3.5, 3.2, color)
            else:
                self.svg.line(x, yy - 3.5, x + 16, yy - 3.5, color, 2.2,
                              "4 3" if kind == "dash" else None)
            self.svg.text(x + 22, yy, label, size, "start", TEXT)


def rgb(color, factor):
    """Mix a '#rrggbb' colour toward the background by 1 - factor."""
    base = [int(BACKGROUND[i:i + 2], 16) for i in (1, 3, 5)]
    c = [int(color[i:i + 2], 16) for i in (1, 3, 5)]
    out = [int(round(b + (v - b) * factor)) for b, v in zip(base, c)]
    return "#%02x%02x%02x" % tuple(out)


def heading(svg, text, subtitle=None, x=24, y=28):
    svg.text(x, y, text, 16, "start", TEXT, weight="700")
    if subtitle:
        svg.text(x, y + 19, subtitle, 11.5, "start", MUTED)
