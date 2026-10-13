"""The hr view's transforms, and a Hertzsprung-Russell diagram of a synthetic machine.

Writes docs/media/figures/hr-blackbody-diagram.svg. Run from the repository root:

    python3 docs/figures/hr-blackbody-diagram.py

Top left: temperature from CPU, T = 2400 K (1 + cpu / 5)^0.47 clamped to 40,000 K (cpu in percent
of one core), with the spectral-class boundaries. Top right: a synthetic population of 173
processes (fixed seed) as a Hertzsprung-Russell diagram, temperature on a reversed log axis and
magnitude m = -2.5 log10(memory / 1 GiB) on a reversed axis, with the Theil-Sen main sequence
fitted to m against log10 T and the luminosity-class boundaries of the specification as lines
parallel to it. Bottom: blackbody colours from 1,000 K to 40,000 K, computed by integrating
Planck's law against the CIE 1931 colour matching functions (the Wyman-Sloan-Shirley 2013
multi-lobe fit, which agrees with the tabulated Planckian locus to about 0.001 in x and y),
converted to linear sRGB, normalised to the brightest channel and sRGB-encoded.
"""

import math
import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_planned as P  # noqa: E402

GIB = 1024.0**3
CLASSES = [("O", 30000.0, 50000.0), ("B", 10000.0, 30000.0), ("A", 7500.0, 10000.0),
           ("F", 6000.0, 7500.0), ("G", 5200.0, 6000.0), ("K", 3700.0, 5200.0),
           ("M", 2400.0, 3700.0)]
LUMINOSITY = [(-6.0, "Ia"), (-4.5, "Ib"), (-3.25, "II"), (-1.75, "III"), (-0.9, "IV"),
              (1.5, "V"), (2.75, "VI")]
C2 = 1.438777e-2


def lobe(wavelength, centre, left, right):
    sigma = left if wavelength < centre else right
    return math.exp(-0.5 * ((wavelength - centre) / sigma) ** 2)


def matching(wavelength):
    x = (1.056 * lobe(wavelength, 599.8, 37.9, 31.0) + 0.362 * lobe(wavelength, 442.0, 16.0, 26.7)
         - 0.065 * lobe(wavelength, 501.1, 20.4, 26.2))
    y = 0.821 * lobe(wavelength, 568.8, 46.9, 40.5) + 0.286 * lobe(wavelength, 530.9, 16.3, 31.1)
    z = 1.217 * lobe(wavelength, 437.0, 11.8, 36.0) + 0.681 * lobe(wavelength, 459.0, 26.0, 13.8)
    return x, y, z


MATCHING = {w: matching(float(w)) for w in range(360, 831)}


def encode(value):
    value = max(value, 0.0)
    return 12.92 * value if value <= 0.0031308 else 1.055 * value ** (1 / 2.4) - 0.055


def blackbody(temperature):
    """sRGB bytes of a blackbody of the given temperature, normalised to its brightest channel."""
    big_x = big_y = big_z = 0.0
    for wavelength, (x, y, z) in MATCHING.items():
        metres = wavelength * 1e-9
        planck = 1.0 / (metres**5 * (math.exp(C2 / (metres * temperature)) - 1.0))
        big_x += planck * x
        big_y += planck * y
        big_z += planck * z
    red = 3.2406 * big_x - 1.5372 * big_y - 0.4986 * big_z
    green = -0.9689 * big_x + 1.8758 * big_y + 0.0415 * big_z
    blue = 0.0557 * big_x - 0.2040 * big_y + 1.0570 * big_z
    red, green, blue = max(red, 0.0), max(green, 0.0), max(blue, 0.0)
    peak = max(red, green, blue)
    return tuple(int(round(255 * encode(c / peak))) for c in (red, green, blue))


def hexcolour(rgb):
    return "#%02x%02x%02x" % rgb


def temperature(cpu_percent):
    return min(2400.0 * (1.0 + cpu_percent / 5.0) ** 0.47, 40000.0)


def magnitude(memory_bytes):
    if memory_bytes <= 0:
        return 9.0
    return min(max(-2.5 * math.log10(memory_bytes / GIB), -6.0), 9.0)


def spectral_class(t):
    for letter, low, high in CLASSES:
        if t >= low or letter == "M":
            fraction = (math.log10(high) - math.log10(t)) / (math.log10(high) - math.log10(low))
            return letter, int(min(max(math.floor(10 * fraction), 0), 9))


def luminosity_class(residual, t):
    if residual >= 2.75:
        return "D" if t >= 7500.0 else "VI"
    for edge, name in LUMINOSITY:
        if residual < edge:
            return name
    return "VI"


def theil_sen(xs, ys, minimum_dx=0.01):
    slopes = []
    for i in range(len(xs)):
        for j in range(i + 1, len(xs)):
            dx = xs[j] - xs[i]
            if abs(dx) > minimum_dx:
                slopes.append((ys[j] - ys[i]) / dx)
    slopes.sort()
    slope = slopes[len(slopes) // 2]
    offsets = sorted(y - slope * x for x, y in zip(xs, ys))
    return offsets[len(offsets) // 2], slope


def population():
    rng = random.Random(1911)
    stars = []
    for _ in range(170):
        cpu = math.exp(rng.uniform(math.log(0.02), math.log(700.0)))
        log_memory = math.log(120.0 * 2**20) + 0.33 * math.log(cpu + 1.0) + rng.gauss(0.0, 1.5)
        stars.append((cpu, math.exp(log_memory)))
    stars.append((1500.0, 18.0 * GIB))
    stars.append((420.0, 0.05 * GIB))
    stars.append((0.0, 6.0 * GIB))
    return stars


def main():
    svg = P.Svg(1180, 600, "Temperature, colour and a Hertzsprung-Russell diagram of processes",
                "Temperature against CPU with spectral classes, a synthetic machine on a "
                "Hertzsprung-Russell diagram with the fitted main sequence, and blackbody colours.")
    svg.text(24, 28, "hr: from CPU and memory to a star", size=16, weight="600")
    svg.text(24, 47, "T = 2400 K (1 + cpu/5)^0.47, m = -2.5 log10(memory / 1 GiB); class from T, "
             "luminosity class from the residual of a Theil-Sen fit.", size=11, fill=P.MUTED)
    top, h = 100, 300

    cpus = [10 ** (-2.0 + 5.5 * k / 200.0) for k in range(201)]
    axes = P.Axes(svg, 70, top, 380, h, (0.01, 3000), (2000, 45000), xlog=True, ylog=True,
                  xlabel="CPU, percent of one core", ylabel="temperature (K)",
                  title="Temperature from CPU", xticks=[0.01, 0.1, 1, 10, 100, 1000],
                  yticks=[2400, 3700, 5200, 7500, 10000, 30000],
                  ytick_labels=["2400", "3700", "5200", "7500", "10000", "30000"],
                  xtick_labels=["0.01", "0.1", "1", "10", "100", "1000"])
    axes.plot(cpus, [temperature(c) for c in cpus], stroke=P.YELLOW, width=2.4)
    for letter, low, high in CLASSES:
        if letter != "M":
            axes.hline(low, dash="2 4", opacity=0.5)
        mid = math.sqrt(low * min(high, 45000.0))
        axes.label(0.011, mid, letter, size=11.5, fill=P.TEXT, dx=4, dy=4)
    axes.dots([100.0], [temperature(100.0)], fill=P.AQUA, radius=4)
    axes.label(100.0, temperature(100.0), "one core: %.0f K (A0)" % temperature(100.0), size=10.5,
               dx=-8, dy=18, anchor="end")

    stars = population()
    xs = [math.log10(temperature(c)) for c, _ in stars]
    ys = [magnitude(m) for _, m in stars]
    intercept, slope = theil_sen(xs, ys)
    axes2 = P.Axes(svg, 560, top, 580, h, (45000, 2250), (9, -6), xlog=True,
                   xlabel="temperature (K), hot on the left", ylabel="magnitude m (bright at the top)",
                   title="A synthetic machine", xticks=[40000, 20000, 10000, 7000, 5000, 3500],
                   yticks=[-6, -3, 0, 3, 6, 9], invert_y=False)
    # The axis limits were given reversed; draw the lines with the same transform.
    lo, hi = math.log10(2250.0), math.log10(45000.0)
    line = [10 ** (lo + (hi - lo) * k / 50) for k in range(51)]
    axes2.plot(line, [intercept + slope * math.log10(t) for t in line], stroke="#e6edf3",
               width=1.6, dash="6 4")
    for edge, name in LUMINOSITY[:-1]:
        axes2.plot(line, [intercept + slope * math.log10(t) + edge for t in line],
                   stroke=P.MUTED, width=0.8, dash="1 4", opacity=0.8)
        axes2.label(2320.0, intercept + slope * math.log10(2320.0) + edge, name, size=10,
                    fill=P.MUTED, dx=6, dy=-2)
    for (cpu, memory), t, m in zip(stars, [10**x for x in xs], ys):
        residual = m - (intercept + slope * math.log10(t))
        axes2.dots([t], [m], fill=hexcolour(blackbody(t)), radius=3.6, stroke=P.BACKGROUND)
    counts = {}
    for t, m in zip([10**x for x in xs], ys):
        name = luminosity_class(m - (intercept + slope * math.log10(t)), t)
        counts[name] = counts.get(name, 0) + 1
    svg.text(570, top + h + 55, "Theil-Sen fit: m = %.2f %+.2f log10 T; classes: %s" % (
        intercept, slope, ", ".join("%s %d" % (k, counts[k]) for k in
                                    ("Ia", "Ib", "II", "III", "IV", "V", "VI", "D") if k in counts)),
        size=10.5, fill=P.MUTED)

    # Blackbody strip.
    strip_top = 480
    svg.text(70, strip_top - 12, "Blackbody colour, 1,000 K to 40,000 K (log scale)", size=13,
             weight="600")
    count = 240
    left, width = 70, 1070
    for k in range(count):
        t = 1000.0 * (40.0 ** (k / (count - 1)))
        svg.rect(left + width * k / count, strip_top, width / count + 0.6, 34,
                 fill=hexcolour(blackbody(t)))
    for t in (1000, 2400, 3700, 5800, 7500, 10000, 20000, 40000):
        x = left + width * math.log(t / 1000.0) / math.log(40.0)
        svg.line(x, strip_top + 34, x, strip_top + 42, stroke=P.AXIS, width=1)
        svg.text(x, strip_top + 56, "%d K" % t, size=10.5, anchor="middle", fill=P.MUTED)
        svg.text(x, strip_top + 70, "%d, %d, %d" % blackbody(float(t)), size=9.5, anchor="middle",
                 fill=P.MUTED)
    svg.save("docs/media/figures/hr-blackbody-diagram.svg")
    print("fit", round(intercept, 3), round(slope, 3), counts)
    for t in (1000, 2400, 6500, 10000, 40000):
        print(t, blackbody(float(t)))


if __name__ == "__main__":
    main()
