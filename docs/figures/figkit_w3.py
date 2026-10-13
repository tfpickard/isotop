"""Tiny stdlib-only SVG kit for the layout, simulation, hashing and colour figures.

Every figure carries its own dark background (#0e1116) with light strokes and text (#c9d1d9), a
viewBox, a <title> and a <desc>. Nothing here is random and nothing reads the clock.
"""

import math

BG = "#0e1116"
INK = "#c9d1d9"
MUTED = "#8b949e"
GRID = "#262c36"
AXIS = "#6e7681"
FONT = "system-ui, sans-serif"
BLUE = "#3987e5"
ORANGE = "#d95926"
AQUA = "#199e70"
YELLOW = "#c98500"
MAGENTA = "#d55181"
VIOLET = "#9085e9"
RED = "#e66767"


def esc(text):
    return str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


def num(value):
    text = "%.2f" % value
    text = text.rstrip("0").rstrip(".") if "." in text else text
    return "0" if text in ("-0", "") else text


class Svg:
    def __init__(self, width, height, title, desc):
        self.width, self.height = width, height
        self.title, self.desc = title, desc
        self.parts = []

    def add(self, markup):
        self.parts.append(markup)

    def rect(self, x, y, w, h, fill="none", stroke=None, width=1, rx=0, opacity=None):
        extra = ' stroke="%s" stroke-width="%s"' % (stroke, num(width)) if stroke else ""
        extra += ' rx="%s"' % num(rx) if rx else ""
        extra += ' fill-opacity="%s"' % num(opacity) if opacity is not None else ""
        self.add('<rect x="%s" y="%s" width="%s" height="%s" fill="%s"%s/>' % (num(x), num(y), num(w), num(h), fill, extra))

    def line(self, x1, y1, x2, y2, stroke=AXIS, width=1, dash=None, opacity=None):
        extra = ' stroke-dasharray="%s"' % dash if dash else ""
        extra += ' stroke-opacity="%s"' % num(opacity) if opacity is not None else ""
        self.add('<line x1="%s" y1="%s" x2="%s" y2="%s" stroke="%s" stroke-width="%s"%s/>' % (num(x1), num(y1), num(x2), num(y2), stroke, num(width), extra))

    def circle(self, x, y, r, fill=INK, stroke=None, width=1, opacity=None):
        extra = ' stroke="%s" stroke-width="%s"' % (stroke, num(width)) if stroke else ""
        extra += ' fill-opacity="%s"' % num(opacity) if opacity is not None else ""
        self.add('<circle cx="%s" cy="%s" r="%s" fill="%s"%s/>' % (num(x), num(y), num(r), fill, extra))

    def polyline(self, points, stroke=INK, width=1.5, dash=None, opacity=None, fill="none"):
        pts = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        extra = ' stroke-dasharray="%s"' % dash if dash else ""
        extra += ' stroke-opacity="%s"' % num(opacity) if opacity is not None else ""
        self.add('<polyline points="%s" fill="%s" stroke="%s" stroke-width="%s" stroke-linejoin="round"%s/>' % (pts, fill, stroke, num(width), extra))

    def text(self, x, y, string, size=12, fill=INK, anchor="start", weight=None, rotate=None, italic=False):
        extra = ' font-weight="%s"' % weight if weight else ""
        extra += ' font-style="italic"' if italic else ""
        extra += ' transform="rotate(%s %s %s)"' % (num(rotate), num(x), num(y)) if rotate else ""
        self.add('<text x="%s" y="%s" font-size="%s" fill="%s" text-anchor="%s" font-family="%s"%s>%s</text>' % (num(x), num(y), num(size), fill, anchor, FONT, extra, esc(string)))

    def save(self, path):
        head = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d" role="img">\n' % (self.width, self.height, self.width, self.height)
        head += "<title>%s</title>\n<desc>%s</desc>\n" % (esc(self.title), esc(self.desc))
        head += '<rect width="%d" height="%d" fill="%s"/>\n' % (self.width, self.height, BG)
        with open(path, "w") as handle:
            handle.write(head + "\n".join(self.parts) + "\n</svg>\n")


class Axes:
    """A plot rectangle with linear or log10 axes; draws grid, ticks and labels."""

    def __init__(self, svg, x, y, w, h, xr, yr, xlog=False, ylog=False):
        self.svg, self.x, self.y, self.w, self.h = svg, x, y, w, h
        self.xr, self.yr, self.xlog, self.ylog = xr, yr, xlog, ylog

    def _t(self, v, r, log):
        lo, hi = r
        if log:
            return (math.log10(v) - math.log10(lo)) / (math.log10(hi) - math.log10(lo))
        return (v - lo) / (hi - lo)

    def px(self, v):
        return self.x + self._t(v, self.xr, self.xlog) * self.w

    def py(self, v):
        return self.y + self.h - self._t(v, self.yr, self.ylog) * self.h

    def frame(self, xticks, yticks, xlabel=None, ylabel=None, xfmt=str, yfmt=str):
        s = self.svg
        for v in yticks:
            s.line(self.x, self.py(v), self.x + self.w, self.py(v), GRID, 1)
            s.text(self.x - 6, self.py(v) + 4, yfmt(v), 11, MUTED, "end")
        for v in xticks:
            s.line(self.px(v), self.y, self.px(v), self.y + self.h, GRID, 1)
            s.text(self.px(v), self.y + self.h + 16, xfmt(v), 11, MUTED, "middle")
        s.rect(self.x, self.y, self.w, self.h, "none", AXIS, 1)
        if xlabel:
            s.text(self.x + self.w / 2, self.y + self.h + 34, xlabel, 12, INK, "middle")
        if ylabel:
            s.text(self.x - 44, self.y + self.h / 2, ylabel, 12, INK, "middle", rotate=-90)

    def path(self, xs, ys, stroke, width=1.5, dash=None, opacity=None):
        pts = [(self.px(a), self.py(b)) for a, b in zip(xs, ys)]
        self.svg.polyline(pts, stroke, width, dash, opacity)
