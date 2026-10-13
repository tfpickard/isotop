"""Tiny stdlib-only SVG plotting kit shared by the City, Orbit, Ripple, Flow and Wave figure scripts.

Every figure carries its own dark background (#0e1116) with light strokes and text (#c9d1d9), a
viewBox, a <title> and a <desc>, so it reads the same on both GitHub themes. Nothing here is random
and nothing reads the clock: running a script twice writes the same bytes.
"""

import base64
import math
import struct
import zlib

BG = "#0e1116"
INK = "#c9d1d9"
MUTED = "#8b949e"
GRID = "#262c36"
AXIS = "#6e7681"
FONT = "system-ui, sans-serif"

# Categorical slots in fixed order (validated on the #0e1116 surface; the first four also
# differ by dash pattern or direct label wherever a figure uses them).
BLUE = "#3987e5"
ORANGE = "#d95926"
AQUA = "#199e70"
YELLOW = "#c98500"
MAGENTA = "#d55181"
VIOLET = "#9085e9"
RED = "#e66767"
SERIES = [BLUE, ORANGE, AQUA, YELLOW, MAGENTA, VIOLET, RED]


def esc(text):
    return (
        str(text).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")
    )


def num(value):
    """Compact, deterministic number for SVG attributes."""
    text = "%.2f" % value
    if "." in text:
        text = text.rstrip("0").rstrip(".")
    return "0" if text in ("-0", "") else text


def tick_label(value):
    if value == 0:
        return "0"
    magnitude = abs(value)
    if magnitude >= 1e5 or magnitude < 1e-3:
        mantissa, exponent = ("%.0e" % value).split("e")
        return "%se%d" % (mantissa, int(exponent))
    text = "%.4g" % value
    return text


def nice_ticks(low, high, target=6):
    span = high - low
    if span <= 0:
        return [low]
    raw = span / max(target, 1)
    power = 10 ** math.floor(math.log10(raw))
    for step in (1, 2, 2.5, 5, 10):
        if raw <= step * power:
            step *= power
            break
    first = math.ceil(low / step - 1e-9) * step
    ticks = []
    value = first
    while value <= high + step * 1e-9:
        ticks.append(round(value, 10))
        value += step
    return ticks


def log_ticks(low, high):
    ticks = []
    exponent = math.floor(math.log10(low))
    while 10**exponent <= high * 1.0000001:
        if 10**exponent >= low * 0.9999999:
            ticks.append(10.0**exponent)
        exponent += 1
    return ticks


class Figure:
    def __init__(self, width, height, title, description):
        self.width = width
        self.height = height
        self.title = title
        self.description = description
        self.parts = []
        self.defs = []
        self.clips = 0

    def add(self, markup):
        self.parts.append(markup)

    def text(self, x, y, content, size=12, anchor="start", fill=INK, weight=None, rotate=None, italic=False, opacity=None):
        attributes = 'x="%s" y="%s" font-size="%s" text-anchor="%s" fill="%s"' % (
            num(x), num(y), num(size), anchor, fill)
        if weight:
            attributes += ' font-weight="%s"' % weight
        if italic:
            attributes += ' font-style="italic"'
        if opacity is not None:
            attributes += ' opacity="%s"' % num(opacity)
        if rotate is not None:
            attributes += ' transform="rotate(%s %s %s)"' % (num(rotate), num(x), num(y))
        self.add("<text %s>%s</text>" % (attributes, esc(content)))

    def line(self, x1, y1, x2, y2, color=INK, width=1.0, dash=None, opacity=None):
        attributes = 'x1="%s" y1="%s" x2="%s" y2="%s" stroke="%s" stroke-width="%s"' % (
            num(x1), num(y1), num(x2), num(y2), color, num(width))
        if dash:
            attributes += ' stroke-dasharray="%s"' % dash
        if opacity is not None:
            attributes += ' stroke-opacity="%s"' % num(opacity)
        self.add("<line %s/>" % attributes)

    def rect(self, x, y, w, h, fill="none", stroke=None, width=1.0, opacity=None, rx=None):
        attributes = 'x="%s" y="%s" width="%s" height="%s" fill="%s"' % (num(x), num(y), num(w), num(h), fill)
        if stroke:
            attributes += ' stroke="%s" stroke-width="%s"' % (stroke, num(width))
        if opacity is not None:
            attributes += ' fill-opacity="%s"' % num(opacity)
        if rx:
            attributes += ' rx="%s"' % num(rx)
        self.add("<rect %s/>" % attributes)

    def circle(self, x, y, r, fill="none", stroke=None, width=1.0, opacity=None):
        attributes = 'cx="%s" cy="%s" r="%s" fill="%s"' % (num(x), num(y), num(r), fill)
        if stroke:
            attributes += ' stroke="%s" stroke-width="%s"' % (stroke, num(width))
        if opacity is not None:
            attributes += ' fill-opacity="%s"' % num(opacity)
        self.add("<circle %s/>" % attributes)

    def polyline(self, points, color=INK, width=1.6, dash=None, fill="none", opacity=None, close=False):
        data = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        tag = "polygon" if close else "polyline"
        attributes = 'points="%s" fill="%s" stroke="%s" stroke-width="%s" stroke-linejoin="round" stroke-linecap="round"' % (
            data, fill, color, num(width))
        if dash:
            attributes += ' stroke-dasharray="%s"' % dash
        if opacity is not None:
            attributes += ' stroke-opacity="%s"' % num(opacity)
        self.add("<%s %s/>" % (tag, attributes))

    def polygon(self, points, fill, opacity=None, stroke=None, width=1.0):
        data = " ".join("%s,%s" % (num(x), num(y)) for x, y in points)
        attributes = 'points="%s" fill="%s"' % (data, fill)
        if opacity is not None:
            attributes += ' fill-opacity="%s"' % num(opacity)
        if stroke:
            attributes += ' stroke="%s" stroke-width="%s"' % (stroke, num(width))
        self.add("<polygon %s/>" % attributes)

    def clip(self, x, y, w, h):
        self.clips += 1
        name = "clip%d" % self.clips
        self.defs.append('<clipPath id="%s"><rect x="%s" y="%s" width="%s" height="%s"/></clipPath>' % (
            name, num(x), num(y), num(w), num(h)))
        return name

    def png(self, x, y, w, h, pixels, columns, rows, smooth=True):
        """Embeds an RGB raster (a flat list of (r, g, b)) stored uncompressed, so the bytes never
        depend on the zlib build."""
        raw = bytearray()
        for row in range(rows):
            raw.append(0)
            for column in range(columns):
                raw.extend(pixels[row * columns + column])
        def chunk(kind, data):
            body = kind + data
            return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
        # zlib stream of stored blocks (level 0): deterministic across implementations
        stored = bytearray(b"\x78\x01")
        data = bytes(raw)
        position = 0
        while position < len(data) or position == 0:
            block = data[position:position + 65535]
            position += len(block)
            final = 1 if position >= len(data) else 0
            stored.append(final)
            stored.extend(struct.pack("<HH", len(block), len(block) ^ 0xFFFF))
            stored.extend(block)
            if final:
                break
        stored.extend(struct.pack(">I", zlib.adler32(data) & 0xFFFFFFFF))
        png = (b"\x89PNG\r\n\x1a\n"
               + chunk(b"IHDR", struct.pack(">IIBBBBB", columns, rows, 8, 2, 0, 0, 0))
               + chunk(b"IDAT", bytes(stored))
               + chunk(b"IEND", b""))
        encoded = base64.b64encode(png).decode("ascii")
        rendering = "" if smooth else ' style="image-rendering:pixelated"'
        self.add('<image x="%s" y="%s" width="%s" height="%s" preserveAspectRatio="none"%s href="data:image/png;base64,%s"/>' % (
            num(x), num(y), num(w), num(h), rendering, encoded))

    def heading(self, content, sub=None):
        self.text(20, 28, content, size=16, weight="600")
        if sub:
            self.text(20, 47, sub, size=12, fill=MUTED)

    def save(self, path):
        head = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d" font-family="%s">' % (
            self.width, self.height, self.width, self.height, FONT)
        body = ["<title>%s</title>" % esc(self.title), "<desc>%s</desc>" % esc(self.description)]
        if self.defs:
            body.append("<defs>%s</defs>" % "".join(self.defs))
        body.append('<rect width="%d" height="%d" fill="%s"/>' % (self.width, self.height, BG))
        body.extend(self.parts)
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(head + "\n" + "\n".join(body) + "\n</svg>\n")


class Axes:
    """A plotting panel: a pixel box, data limits (linear or log) and the helpers to draw in data
    coordinates."""

    def __init__(self, figure, x, y, w, h, xlim, ylim, xlog=False, ylog=False, xlabel="", ylabel="",
                 title="", xticks=None, yticks=None, xformat=None, yformat=None, grid=True,
                 xminor=False, yminor=False):
        self.figure = figure
        self.x, self.y, self.w, self.h = x, y, w, h
        self.xlim, self.ylim = xlim, ylim
        self.xlog, self.ylog = xlog, ylog
        self.clip_name = figure.clip(x, y, w, h)
        xt = xticks if xticks is not None else (log_ticks(*xlim) if xlog else nice_ticks(*xlim))
        yt = yticks if yticks is not None else (log_ticks(*ylim) if ylog else nice_ticks(*ylim))
        xf = xformat or tick_label
        yf = yformat or tick_label
        if grid:
            for value in xt:
                if xlim[0] <= value <= xlim[1]:
                    figure.line(self.X(value), y, self.X(value), y + h, GRID, 1)
            for value in yt:
                if ylim[0] <= value <= ylim[1]:
                    figure.line(x, self.Y(value), x + w, self.Y(value), GRID, 1)
            if xlog and xminor:
                self.minor_grid(True)
            if ylog and yminor:
                self.minor_grid(False)
        figure.rect(x, y, w, h, "none", AXIS, 1)
        for value in xt:
            if xlim[0] <= value <= xlim[1]:
                figure.line(self.X(value), y + h, self.X(value), y + h + 4, AXIS, 1)
                figure.text(self.X(value), y + h + 17, xf(value), 11, "middle", MUTED)
        for value in yt:
            if ylim[0] <= value <= ylim[1]:
                figure.line(x - 4, self.Y(value), x, self.Y(value), AXIS, 1)
                figure.text(x - 8, self.Y(value) + 4, yf(value), 11, "end", MUTED)
        if xlabel:
            figure.text(x + w / 2, y + h + 36, xlabel, 12, "middle", INK)
        if ylabel:
            figure.text(x - 46, y + h / 2, ylabel, 12, "middle", INK, rotate=-90)
        if title:
            figure.text(x, y - 10, title, 13, "start", INK, weight="600")

    def minor_grid(self, horizontal_axis):
        figure = self.figure
        low, high = (self.xlim if horizontal_axis else self.ylim)
        exponent = math.floor(math.log10(low))
        while 10**exponent <= high:
            for m in range(2, 10):
                value = m * 10**exponent
                if low <= value <= high:
                    if horizontal_axis:
                        figure.line(self.X(value), self.y, self.X(value), self.y + self.h, GRID, 0.5, opacity=0.6)
                    else:
                        figure.line(self.x, self.Y(value), self.x + self.w, self.Y(value), GRID, 0.5, opacity=0.6)
            exponent += 1

    def X(self, value):
        low, high = self.xlim
        if self.xlog:
            t = (math.log10(value) - math.log10(low)) / (math.log10(high) - math.log10(low))
        else:
            t = (value - low) / (high - low)
        return self.x + t * self.w

    def Y(self, value):
        low, high = self.ylim
        if self.ylog:
            t = (math.log10(value) - math.log10(low)) / (math.log10(high) - math.log10(low))
        else:
            t = (value - low) / (high - low)
        return self.y + self.h - t * self.h

    def plot(self, xs, ys, color=BLUE, width=1.8, dash=None, opacity=None):
        points = []
        for xv, yv in zip(xs, ys):
            if yv is None or xv is None:
                continue
            if (self.xlog and xv <= 0) or (self.ylog and yv <= 0):
                continue
            points.append((self.X(xv), self.Y(yv)))
        # a clip group keeps curves inside the panel when they leave the limits
        self.figure.add('<g clip-path="url(#%s)">' % self.clip_name)
        self.figure.polyline(points, color, width, dash, opacity=opacity)
        self.figure.add("</g>")

    def area(self, xs, lows, highs, color=BLUE, opacity=0.18):
        top = [(self.X(a), self.Y(b)) for a, b in zip(xs, highs)]
        bottom = [(self.X(a), self.Y(b)) for a, b in zip(reversed(xs), reversed(lows))]
        self.figure.add('<g clip-path="url(#%s)">' % self.clip_name)
        self.figure.polygon(top + bottom, color, opacity)
        self.figure.add("</g>")

    def scatter(self, xs, ys, color=BLUE, radius=3.0, opacity=None, stroke=BG):
        self.figure.add('<g clip-path="url(#%s)">' % self.clip_name)
        for xv, yv in zip(xs, ys):
            self.figure.circle(self.X(xv), self.Y(yv), radius, color, stroke, 1.0, opacity)
        self.figure.add("</g>")

    def hline(self, value, color=MUTED, width=1.0, dash="4 3", label=None, label_x=None, anchor="start"):
        self.figure.line(self.x, self.Y(value), self.x + self.w, self.Y(value), color, width, dash)
        if label:
            lx = self.x + 6 if label_x is None else self.X(label_x)
            self.figure.text(lx, self.Y(value) - 4, label, 11, anchor, color)

    def vline(self, value, color=MUTED, width=1.0, dash="4 3", label=None, label_y=None, anchor="start"):
        self.figure.line(self.X(value), self.y, self.X(value), self.y + self.h, color, width, dash)
        if label:
            ly = self.y + 14 if label_y is None else self.Y(label_y)
            self.figure.text(self.X(value) + 4, ly, label, 11, anchor, color)

    def vband(self, low, high, color=MUTED, opacity=0.14):
        left, right = self.X(max(low, self.xlim[0])), self.X(min(high, self.xlim[1]))
        self.figure.rect(left, self.y, right - left, self.h, color, None, 1, opacity)

    def label(self, xv, yv, content, size=11, anchor="start", fill=INK, dx=0, dy=0, weight=None):
        self.figure.text(self.X(xv) + dx, self.Y(yv) + dy, content, size, anchor, fill, weight)

    def legend(self, entries, corner="tr", pad=8, columns=1):
        """entries: (label, color, dash) triples drawn as short line samples with the text in ink."""
        row = 16
        width = max(len(entry[0]) for entry in entries) * 6.4 + 38
        height = row * len(entries) + 8
        if corner == "tr":
            x0, y0 = self.x + self.w - width - pad, self.y + pad
        elif corner == "tl":
            x0, y0 = self.x + pad, self.y + pad
        elif corner == "br":
            x0, y0 = self.x + self.w - width - pad, self.y + self.h - height - pad
        else:
            x0, y0 = self.x + pad, self.y + self.h - height - pad
        self.figure.rect(x0, y0, width, height, BG, GRID, 1, 0.85)
        for index, (label, color, dash) in enumerate(entries):
            cy = y0 + 14 + index * row
            self.figure.line(x0 + 7, cy - 4, x0 + 27, cy - 4, color, 2.2, dash)
            self.figure.text(x0 + 33, cy, label, 11, "start", INK)
