"""Minimal SVG plotting for the sampling, model, life-event and history figures.

Standard library only and deterministic: the same call always writes the same bytes. Every
figure carries its own dark background (#0e1116) with light strokes (#c9d1d9), so it reads on
both GitHub themes. Used by the model-*, life-* and history-* scripts in this directory.
"""

import math
from xml.sax.saxutils import escape

BACKGROUND = "#0e1116"
INK = "#c9d1d9"
MUTED = "#8b949e"
GRID = "#262c36"
FONT = "system-ui, sans-serif"
BLUE = "#58a6ff"
ORANGE = "#f0883e"
GREEN = "#7ee787"
PURPLE = "#d2a8ff"
YELLOW = "#e3b341"
RED = "#ff7b72"
CYAN = "#76e3ea"


def num(value):
    """A short, stable decimal for an SVG coordinate."""
    text = f"{value:.2f}"
    return text.rstrip("0").rstrip(".") if "." in text else text


class Figure:
    """An SVG document with a title and any number of `Axes`."""

    def __init__(self, width, height, title, description=""):
        self.width = width
        self.height = height
        self.title = title
        self.description = description
        self.body = []

    def add(self, markup):
        self.body.append(markup)

    def text(self, x, y, string, size=13, anchor="start", color=INK, weight="normal", rotate=None, style=None):
        transform = f' transform="rotate({rotate} {num(x)} {num(y)})"' if rotate is not None else ""
        italic = f' font-style="{style}"' if style else ""
        self.add(
            f'<text x="{num(x)}" y="{num(y)}" font-size="{size}" fill="{color}" '
            f'text-anchor="{anchor}" font-weight="{weight}"{italic}{transform}>{escape(string)}</text>'
        )

    def axes(self, *args, **kwargs):
        return Axes(self, *args, **kwargs)

    def render(self):
        head = (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {self.width} {self.height}" '
            f'width="{self.width}" height="{self.height}" font-family="{FONT}" role="img">\n'
            f"<title>{escape(self.title)}</title>\n"
        )
        if self.description:
            head += f"<desc>{escape(self.description)}</desc>\n"
        head += f'<rect width="100%" height="100%" fill="{BACKGROUND}"/>\n'
        return head + "\n".join(self.body) + "\n</svg>\n"

    def save(self, path):
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(self.render())


def ticks_linear(low, high, target=6):
    span = high - low
    raw = span / max(target, 1)
    magnitude = 10 ** math.floor(math.log10(raw))
    for factor in (1, 2, 2.5, 5, 10):
        step = factor * magnitude
        if span / step <= target + 1:
            break
    first = math.ceil(low / step - 1e-9) * step
    values = []
    value = first
    while value <= high + 1e-9:
        values.append(round(value, 10))
        value += step
    return values


def ticks_log(low, high):
    values = []
    exponent = math.floor(math.log10(low))
    while 10**exponent <= high * 1.0000001:
        for mantissa in (1, 2, 5):
            value = mantissa * 10**exponent
            if low * 0.9999999 <= value <= high * 1.0000001:
                values.append(value)
        exponent += 1
    return values


def label_of(value):
    if value == 0:
        return "0"
    magnitude = abs(value)
    if magnitude >= 1000 or magnitude < 0.01:
        exponent = math.floor(math.log10(magnitude))
        mantissa = value / 10**exponent
        if abs(mantissa - round(mantissa)) < 1e-9:
            return f"{int(round(mantissa))}e{exponent}" if round(mantissa) != 1 else f"1e{exponent}"
    text = f"{value:.6g}"
    return text


class Axes:
    """A plotting rectangle with its own data limits and optional log scales."""

    def __init__(
        self,
        figure,
        left,
        top,
        width,
        height,
        xlim,
        ylim,
        xlabel="",
        ylabel="",
        title="",
        xlog=False,
        ylog=False,
        xticks=None,
        yticks=None,
        xformat=None,
        yformat=None,
    ):
        self.figure = figure
        self.left, self.top, self.width, self.height = left, top, width, height
        self.xlim, self.ylim = xlim, ylim
        self.xlog, self.ylog = xlog, ylog
        self.clip = f"clip{len(figure.body)}"
        figure.add(
            f'<clipPath id="{self.clip}"><rect x="{num(left)}" y="{num(top)}" '
            f'width="{num(width)}" height="{num(height)}"/></clipPath>'
        )
        xs = xticks if xticks is not None else (ticks_log(*xlim) if xlog else ticks_linear(*xlim))
        ys = yticks if yticks is not None else (ticks_log(*ylim) if ylog else ticks_linear(*ylim))
        for value in xs:
            x = self.px(value)
            figure.add(f'<line x1="{num(x)}" y1="{num(top)}" x2="{num(x)}" y2="{num(top + height)}" stroke="{GRID}" stroke-width="1"/>')
            text = xformat(value) if xformat else label_of(value)
            figure.text(x, top + height + 16, text, size=11, anchor="middle", color=MUTED)
        for value in ys:
            y = self.py(value)
            figure.add(f'<line x1="{num(left)}" y1="{num(y)}" x2="{num(left + width)}" y2="{num(y)}" stroke="{GRID}" stroke-width="1"/>')
            text = yformat(value) if yformat else label_of(value)
            figure.text(left - 7, y + 4, text, size=11, anchor="end", color=MUTED)
        figure.add(
            f'<rect x="{num(left)}" y="{num(top)}" width="{num(width)}" height="{num(height)}" '
            f'fill="none" stroke="{MUTED}" stroke-width="1"/>'
        )
        if xlabel:
            figure.text(left + width / 2, top + height + 38, xlabel, size=12, anchor="middle")
        if ylabel:
            figure.text(left - 46, top + height / 2, ylabel, size=12, anchor="middle", rotate=-90)
        if title:
            figure.text(left, top - 10, title, size=13, weight="600")

    def px(self, x):
        low, high = self.xlim
        if self.xlog:
            fraction = (math.log10(x) - math.log10(low)) / (math.log10(high) - math.log10(low))
        else:
            fraction = (x - low) / (high - low)
        return self.left + fraction * self.width

    def py(self, y):
        low, high = self.ylim
        if self.ylog:
            fraction = (math.log10(max(y, 1e-300)) - math.log10(low)) / (math.log10(high) - math.log10(low))
        else:
            fraction = (y - low) / (high - low)
        return self.top + self.height - fraction * self.height

    def path(self, xs, ys, color=BLUE, width=1.8, dash=None, opacity=1.0):
        points = " ".join(f"{num(self.px(x))},{num(self.py(y))}" for x, y in zip(xs, ys))
        dashes = f' stroke-dasharray="{dash}"' if dash else ""
        self.figure.add(
            f'<polyline points="{points}" fill="none" stroke="{color}" stroke-width="{width}" '
            f'stroke-linejoin="round" stroke-linecap="round" opacity="{opacity}"{dashes} clip-path="url(#{self.clip})"/>'
        )

    def steps(self, xs, ys, color=ORANGE, width=1.8, dash=None):
        """A staircase: ys[i] holds from xs[i] to xs[i + 1] (the last one for one more step)."""
        px, py = [], []
        for index, (x, y) in enumerate(zip(xs, ys)):
            nxt = xs[index + 1] if index + 1 < len(xs) else x + (xs[-1] - xs[-2])
            px += [x, nxt]
            py += [y, y]
        self.path(px, py, color, width, dash)

    def points(self, xs, ys, color=GREEN, radius=3.2, opacity=1.0):
        for x, y in zip(xs, ys):
            self.figure.add(
                f'<circle cx="{num(self.px(x))}" cy="{num(self.py(y))}" r="{radius}" fill="{color}" opacity="{opacity}" clip-path="url(#{self.clip})"/>'
            )

    def vline(self, x, color=MUTED, dash="4 4", width=1.2):
        self.figure.add(
            f'<line x1="{num(self.px(x))}" y1="{num(self.top)}" x2="{num(self.px(x))}" y2="{num(self.top + self.height)}" '
            f'stroke="{color}" stroke-width="{width}" stroke-dasharray="{dash}"/>'
        )

    def hline(self, y, color=MUTED, dash="4 4", width=1.2):
        self.figure.add(
            f'<line x1="{num(self.left)}" y1="{num(self.py(y))}" x2="{num(self.left + self.width)}" y2="{num(self.py(y))}" '
            f'stroke="{color}" stroke-width="{width}" stroke-dasharray="{dash}"/>'
        )

    def note(self, x, y, string, size=11, anchor="start", color=INK, dx=0, dy=0):
        self.figure.text(self.px(x) + dx, self.py(y) + dy, string, size=size, anchor=anchor, color=color)

    def legend(self, entries, x=None, y=None, row=17):
        """Entries are (colour, label, dash) tuples; placed in axes pixels from the top left."""
        x = self.left + 12 if x is None else x
        y = self.top + 16 if y is None else y
        for index, (color, label, dash) in enumerate(entries):
            line_y = y + index * row
            dashes = f' stroke-dasharray="{dash}"' if dash else ""
            self.figure.add(
                f'<line x1="{num(x)}" y1="{num(line_y - 4)}" x2="{num(x + 22)}" y2="{num(line_y - 4)}" '
                f'stroke="{color}" stroke-width="2.2"{dashes}/>'
            )
            self.figure.text(x + 29, line_y, label, size=11.5)
