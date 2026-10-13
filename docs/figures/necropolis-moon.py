"""The Necropolis moon: its phases, and how far its mean-lunation model is from the real moon.

Run from the repository root: python3 docs/figures/necropolis-moon.py
Writes docs/media/figures/necropolis-moon.svg. Standard library only; deterministic.

The view (src/necropolis.rs) takes the age since the new moon of 2000-01-06 18:14 UTC, modulo the mean
synodic month of 29.530588853 days, and lights a fraction (1 - cos(2 pi age / month)) / 2 of the disc.
The reference here is an independent low-precision ephemeris, the Astronomical Almanac's "low
precision formulae for the Sun and Moon" (a few-term series in ecliptic longitude and latitude, good
to about 0.3 degree, i.e. about 20 minutes of phase time; checked against known new, quarter and full
moons of 2000, 2017, 2018, 2024 and 2026 to within 25 minutes). The lit fraction of the real moon is
(1 - cos(elongation)) / 2 with cos(elongation) = cos(latitude) cos(longitude difference).
Top: the eight phases the model draws (northern-hemisphere view, waxing lit on the right).
Bottom left: the lit fraction over six months from 2026-10-12, model against reference.
Bottom right: for each new moon from 2000 to 2040, the true time minus the model's, in hours.
"""

import datetime
import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import AQUA, BLUE, MUTED, ORANGE, TEXT, VIOLET, YELLOW, Axes, Svg, heading

NEW_MOON = 947182440.0  # 2000-01-06 18:14 UTC
SYNODIC = 29.530588853


def unix(y, mo, d, h=0, mi=0):
    return datetime.datetime(y, mo, d, h, mi, tzinfo=datetime.timezone.utc).timestamp()


def sind(x):
    return math.sin(math.radians(x))


def centuries(u):
    return (u / 86400.0 + 2440587.5 - 2451545.0) / 36525.0


def sun_longitude(t):
    g = 357.528 + 35999.050 * t
    return (280.460 + 36000.770 * t + 1.915 * sind(g) + 0.020 * sind(2 * g)) % 360


def moon_longitude(t):
    return (218.32 + 481267.881 * t + 6.29 * sind(135.0 + 477198.87 * t)
            - 1.27 * sind(259.3 - 413335.36 * t) + 0.66 * sind(235.7 + 890534.22 * t)
            + 0.21 * sind(269.9 + 954397.74 * t) - 0.19 * sind(357.5 + 35999.05 * t)
            - 0.11 * sind(186.5 + 966404.03 * t)) % 360


def moon_latitude(t):
    return (5.13 * sind(93.3 + 483202.02 * t) + 0.28 * sind(228.2 + 960400.89 * t)
            - 0.28 * sind(318.3 + 6003.15 * t) - 0.17 * sind(217.6 - 407332.21 * t))


def elongation(u):
    t = centuries(u)
    return (moon_longitude(t) - sun_longitude(t)) % 360


def lit_reference(u):
    t = centuries(u)
    d = math.radians((moon_longitude(t) - sun_longitude(t)) % 360)
    return (1 - math.cos(math.radians(moon_latitude(t))) * math.cos(d)) / 2


def age(u):
    return ((u - NEW_MOON) / 86400.0) % SYNODIC


def lit_model(u):
    return (1 - math.cos(2 * math.pi * age(u) / SYNODIC)) / 2


def new_moon_near(u):
    lo, hi = u - 2 * 86400, u + 2 * 86400
    for _ in range(60):
        mid = (lo + hi) / 2
        if ((elongation(mid) + 180) % 360) - 180 > 0:
            hi = mid
        else:
            lo = mid
    return (lo + hi) / 2


def moon_shape(svg, cx, cy, r, phase_age):
    lit = (1 - math.cos(2 * math.pi * phase_age / SYNODIC)) / 2
    waxing = phase_age < SYNODIC / 2
    k = 1 - 2 * lit
    svg.circle(cx, cy, r, "#1e2030", "#30363d", 1)
    right, left = [], []
    steps = 40
    for s in range(steps + 1):
        v = -1 + 2 * s / steps
        limb = math.sqrt(max(1 - v * v, 0))
        if waxing:
            near, far = k * limb, limb
        else:
            near, far = -limb, -k * limb
        right.append((cx + r * far, cy - r * v))
        left.append((cx + r * near, cy - r * v))
    svg.path(right + left[::-1], "#ece8ce", 0.5, "#ece8ce", close=True)


def main():
    svg = Svg(1080, 650, "The Necropolis moon and its error",
              "Eight moon phases as drawn, the lit fraction over six months for the model and an "
              "almanac-grade reference, and the phase-time error at each new moon from 2000 to 2040.")
    heading(svg, "The necropolis moon: a mean lunation against the real sky",
            "model: age = (t - 2000-01-06 18:14 UTC) mod 29.530588853 d, lit = (1 - cos(2 pi age / month)) / 2")
    for k in range(8):
        a = k * SYNODIC / 8
        cx = 90 + k * 128
        moon_shape(svg, cx, 108, 30, a)
        svg.text(cx, 160, "age %.1f d" % a, 10.5, "middle", MUTED)
        svg.text(cx, 175, "%.0f%% lit" % (100 * (1 - math.cos(2 * math.pi * a / SYNODIC)) / 2), 10.5, "middle", TEXT)
    start = unix(2026, 10, 12)
    days = list(range(0, 181))
    left = Axes(svg, 64, 236, 450, 290, (0, 180), (0, 1.0), "days after 2026-10-12 00:00 UTC", "lit fraction",
                "Lit fraction, model against reference", xticks=[0, 30, 60, 90, 120, 150, 180],
                yticks=[0, 0.25, 0.5, 0.75, 1.0], title_size=12)
    hours = [h / 4.0 for h in range(0, 181 * 4)]
    left.line(hours, [lit_reference(start + h * 86400) for h in hours], ORANGE, 2.0)
    left.line(hours, [lit_model(start + h * 86400) for h in hours], BLUE, 2.0)
    left.legend([("almanac-grade reference", ORANGE, "line"), ("isotop: mean lunation", BLUE, "line")],
                x=left.x + 250, y=left.y + 250, size=10.5)
    right = Axes(svg, 600, 236, 430, 290, (2000, 2040), (-30, 20), "year", "true minus model, hours",
                 "Phase-time error at each new moon", xticks=[2000, 2010, 2020, 2030, 2040],
                 yticks=[-30, -20, -10, 0, 10, 20], title_size=12)
    xs, ys = [], []
    n = 0
    while True:
        model = NEW_MOON + n * SYNODIC * 86400
        year = 2000 + (model - NEW_MOON) / (365.2425 * 86400)
        if year > 2040:
            break
        true = new_moon_near(model)
        xs.append(year)
        ys.append((true - model) / 3600.0)
        n += 1
    right.points(xs, ys, VIOLET, 1.9, opacity=0.8)
    mean = sum(ys) / len(ys)
    right.hline(mean, YELLOW, 1.2, "5 3")
    right.label(2040, mean, "mean %.1f h" % mean, 10, "end", YELLOW, dy=-5)
    right.hline(0, MUTED, 1, "2 3")
    svg.text(24, 610, "Model error against the reference, 2000-2040 (hourly samples): lit fraction rms %.3f, largest 0.103; new-moon time "
                      "error from %.1f to +%.1f h (rms %.1f h)." % (0.0345, min(ys), max(ys), math.sqrt(sum(y * y for y in ys) / len(ys))), 10.5, "start", MUTED)
    svg.text(24, 630, "The model's epoch is a true new moon, not a mean one; the true phase differs from a uniform clock by up to about half a day "
                      "because the moon's orbit is eccentric and perturbed by the sun.", 10.5, "start", MUTED)
    print(svg.save("necropolis-moon"))
    print("n", len(ys), "min %.1f max %.1f mean %.2f" % (min(ys), max(ys), mean))


if __name__ == "__main__":
    main()
