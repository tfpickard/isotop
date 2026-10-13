"""Planck spectra, the ironbow palette, a heat-equation die and its screened-Poisson profile.

Run from the repository root: python3 docs/figures/die-thermal.py   (about 30 s)
Writes docs/media/figures/die-thermal.svg.

(a) Planck's law for 300 to 390 K with Wien's peaks and the 8 to 14 micrometre band of an LWIR camera.
(b) The ironbow stops of docs/mockups/code/die.py (the issue takes them from it) and their
    Rec. 709 luma: monotonic, so the palette survives greyscale.
(c) dT/dt = kappa lap(T) + gain P - (T - T_amb)/tau on a 96 x 48 grid (kappa = 22.5 cells^2/s, tau = 2 s,
    step h = 0.2 / kappa, so h * kappa = 0.2 < 1/4), zero-flux edges, a toy hybrid floorplan with one P-core at
    full proxy and the rest near idle; gain calibrated so a single P-core at full proxy settles 40 K
    above ambient, as in the prototype. Shown after 5 tau. The issue's die grid is 192 x 96 with
    kappa = 90, tau = 2 s; this figure halves the grid and divides kappa by 4 to stay quick.
(d) The steady response of the same equation to a point source follows K0(r / lambda) with
    lambda = sqrt(kappa tau): the simulated profile against the Bessel function.
"""

import math

import svgkit_w8 as kit

H_PLANCK, C_LIGHT, K_BOLTZ = 6.62607015e-34, 2.99792458e8, 1.380649e-23
WIEN_B = 2897.771955  # um K
NX, NY = 96, 48
KAPPA, TAU, T_AMB = 22.5, 2.0, 34.0
STEP = 0.2 / KAPPA


def exitance(wavelength_um, temperature):
    lam = wavelength_um * 1e-6
    return (2 * math.pi * H_PLANCK * C_LIGHT ** 2 / lam ** 5
            / (math.exp(H_PLANCK * C_LIGHT / (lam * K_BOLTZ * temperature)) - 1.0)) * 1e-6


def bessel_k0(x):
    total, dt = 0.0, 0.01
    t = dt / 2
    while t < 12:
        total += math.exp(-x * math.cosh(t)) * dt
        t += dt
    return total


def laplacian_step(field, source, kappa=KAPPA, tau=TAU, step=STEP, nx=NX, ny=NY):
    out = []
    for y in range(ny):
        up = field[max(y - 1, 0)]
        down = field[min(y + 1, ny - 1)]
        row = field[y]
        left = [row[0]] + row[:-1]
        right = row[1:] + [row[-1]]
        src = source[y]
        out.append([t + step * (kappa * (a + b + c + d - 4.0 * t) + s - (t - T_AMB) / tau)
                    for t, a, b, c, d, s in zip(row, up, down, left, right, src)])
    return out


def blocks():
    """Toy hybrid die: (name, kind, x0, y0, x1, y1) in cells."""
    result = []
    for i in range(4):
        result.append(("P%d" % i, "P", 14 + i * 14, 4, 14 + i * 14 + 12, 22))
    for c in range(2):
        for k in range(4):
            x0 = 14 + c * 28 + k * 7
            result.append(("E%d" % (c * 4 + k), "E", x0, 28, x0 + 6, 42))
    result.append(("SA", "SA", 2, 4, 11, 42))
    result.append(("GT", "GT", 72, 4, 93, 42))
    return result


def main():
    svg = kit.Svg(960, 700, "Planck, ironbow, a die heat model and its Bessel profile",
                  "Blackbody spectra with the LWIR band, the ironbow palette and its luma, a simulated "
                  "die heat field, and its point-source profile against K0.")
    svg.title_text("die (planned): from blackbody radiation to a screened heat equation")
    # (a) Planck
    ax = kit.Axes(svg, 60, 60, 400, 240, (2, 20), (0, 140), xlabel="wavelength (micrometres)",
                  ylabel="spectral exitance (W m^-2 um^-1)", title="a  blackbody exitance at 300 to 390 K",
                  xticks=[2, 5, 8, 11, 14, 17, 20])
    ax.band(8, 14, color=kit.YELLOW, opacity=0.13)
    lams = [2 + 18 * i / 300 for i in range(301)]
    entries = []
    for temperature, colour in ((300, kit.BLUE), (330, kit.AQUA), (360, kit.ORANGE), (390, kit.RED)):
        ax.line(lams, [exitance(l, temperature) for l in lams], color=colour, width=1.9)
        entries.append(("%d K (%d C), peak %.1f um" % (temperature, temperature - 273, WIEN_B / temperature),
                        colour, None))
    ax.legend(entries, corner="tr", size=10)
    ax.label(8.2, 128, "8 to 14 um", color=kit.YELLOW, size=10)
    # (b) ironbow
    bx0, by0 = 540, 76
    stops = kit.IRONBOW
    bar = [[gradient_t / 255.0 for gradient_t in range(256)]]
    svg.image(bx0, by0, 380, 34, kit.heat_png(bar * 2, stops, 0.0, 1.0), smooth=True)
    svg.rect(bx0, by0, 380, 34, stroke=kit.AXIS)
    svg.text(bx0 + 190, 50, "b  ironbow, 30 to 105 C, and its luma", size=13, anchor="middle", weight="600")
    for value in (30, 45, 60, 75, 90, 105):
        px = bx0 + (value - 30) / 75.0 * 380
        svg.line(px, by0 + 34, px, by0 + 40, stroke=kit.MUTED)
        svg.text(px, by0 + 54, str(value), size=10, anchor="middle", fill=kit.MUTED)
    lx = kit.Axes(svg, bx0, 190, 380, 110, (30, 105), (0, 255), xlabel="temperature (C)",
                  ylabel="luma", xticks=[30, 45, 60, 75, 90, 105], yticks=[0, 128, 255])
    temps = [30 + 75 * i / 100.0 for i in range(101)]
    luma = []
    for i in range(101):
        r, g, b = kit.gradient(stops, i / 100.0)
        luma.append(0.2126 * r + 0.7152 * g + 0.0722 * b)
    lx.line(temps, luma, color=kit.TEXT, width=2.0)
    # (c) heat field
    layout = blocks()
    def power_map(levels):
        source = [[0.0] * NX for _ in range(NY)]
        for name, kind, x0, y0, x1, y1 in layout:
            level = levels.get(name, 0.0)
            area = (x1 - x0) * (y1 - y0)
            for y in range(y0, y1):
                for x in range(x0, x1):
                    source[y][x] = level / area
        return source

    def settle(source, seconds):
        field = [[T_AMB] * NX for _ in range(NY)]
        for _ in range(int(seconds / STEP)):
            field = laplacian_step(field, source)
        return field

    unit = settle(power_map({"P0": 1.0}), 5 * TAU)
    gain = 40.0 / (max(max(r) for r in unit) - T_AMB)
    levels = {"P0": 1.0, "P1": 0.08, "P2": 0.08, "P3": 0.08}
    for k in range(8):
        levels["E%d" % k] = 0.15
    levels["SA"] = 0.2
    levels["GT"] = 0.25
    field = settle([[gain * v for v in row] for row in power_map(levels)], 5 * TAU)
    peak = max(max(r) for r in field)
    scale = 4.0
    cx0, cy0 = 60, 400
    svg.text(cx0 + NX * scale / 2, 386, "c  heat field after 5 tau, one P-core busy", size=13, anchor="middle",
             weight="600")
    svg.image(cx0, cy0, NX * scale, NY * scale, kit.heat_png(field, stops, 30.0, 105.0), smooth=True)
    for name, kind, x0, y0, x1, y1 in layout:
        svg.rect(cx0 + x0 * scale, cy0 + y0 * scale, (x1 - x0) * scale, (y1 - y0) * scale, stroke="#dde3f0",
                 width=0.8, opacity=0.0)
        if kind in ("P", "SA", "GT"):
            svg.text(cx0 + (x0 + 1) * scale, cy0 + (y0 + 3) * scale, name, size=9, fill="#ffffff")
    svg.rect(cx0, cy0, NX * scale, NY * scale, stroke=kit.AXIS)
    svg.text(cx0, cy0 + NY * scale + 18, "peak %.1f C (ambient %.0f C); schematic blocks, not to scale" % (peak, T_AMB),
             size=10.5, fill=kit.MUTED)
    # (d) point source
    size = 64
    source = [[0.0] * size for _ in range(size)]
    source[size // 2][size // 2] = 40.0
    field2 = [[T_AMB] * size for _ in range(size)]
    for _ in range(int(5 * TAU / STEP)):
        field2 = laplacian_step(field2, source, nx=size, ny=size)
    profile = [(r, field2[size // 2][size // 2 + r] - T_AMB) for r in range(0, 28)]
    lam = math.sqrt(KAPPA * TAU)
    dxs = kit.Axes(svg, 560, 400, 360, 200, (0, 27), (1e-3, 1.5), ylog=True, xlabel="distance from the source (cells)",
                   ylabel="T - T_amb (normalised)", title="d  point-source profile against K0(r / lambda)",
                   xticks=[0, 5, 10, 15, 20, 25])
    norm = profile[3][1] / bessel_k0(3 / lam)
    rs = [0.7 + 0.1 * i for i in range(0, 260)]
    dxs.line(rs, [bessel_k0(r / lam) * norm / profile[3][1] for r in rs], color=kit.YELLOW, width=2.0)
    dxs.points([r for r, _ in profile[1:]], [v / profile[3][1] for _, v in profile[1:]], color=kit.AQUA, r=3)
    dxs.legend([("simulation", kit.AQUA, None), ("K0(r/lambda), lambda = %.1f cells" % lam, kit.YELLOW, None)],
               corner="tr", size=10)
    svg.text(480, 686, "kappa and tau are the model's, not silicon's: real silicon spreads heat about 90 times faster per cell",
             size=10.5, anchor="middle", fill=kit.MUTED)
    svg.save("die-thermal")
    print("gain %.2f K per unit power-share; peak %.1f C" % (gain, peak))
    print("lambda = %.2f cells" % lam)
    ratio = [(r, v / bessel_k0(r / lam)) for r, v in profile[3:20:4]]
    print("simulation / K0 (should be constant):", [(r, round(q / ratio[0][1], 3)) for r, q in ratio])


if __name__ == "__main__":
    main()
