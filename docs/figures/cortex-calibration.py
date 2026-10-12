"""From CPU to firing rate: the input-current calibration specified for the cortex view.

Run from the repository root: python3 docs/figures/cortex-calibration.py
Writes docs/media/figures/cortex-calibration.svg.

Issue #52: per neuron type the rheobase rh is the smallest I giving at least 1 Hz and I40 the I
giving 40 Hz, both noise-free; the smoothed cpu / 100 (a fraction of one core) maps piecewise
linearly from [0, 0.005, 0.05, 1, 3] to [0, 0.62 rh, 0.98 rh, I40, I40 + 2 (I40 - 0.98 rh)], clamped
beyond 3, and current noise of sigma 1.2 per 0.5 ms step is added only when I > 0. This script
reproduces that with the Izhikevich neurons of docs/figures/izh_w8.py and reports the firing rate
that results (4 s after 0.5 s of settling, fixed seed).
"""

import math

import izh_w8 as izh
import svgkit_w8 as kit

KINDS = [("FS", "kernel", kit.ORANGE), ("RS", "system", kit.AQUA), ("CH", "session", kit.YELLOW),
         ("IB", "container", kit.VIOLET)]
BREAKS = [0.0, 0.005, 0.05, 1.0, 3.0]


def current(rh, i40, fraction):
    ys = [0.0, 0.62 * rh, 0.98 * rh, i40, i40 + 2.0 * (i40 - 0.98 * rh)]
    c = min(max(fraction, 0.0), 3.0)
    for a, b, ya, yb in zip(BREAKS, BREAKS[1:], ys, ys[1:]):
        if c <= b:
            return ya + (yb - ya) * (c - a) / (b - a)
    return ys[-1]


def main():
    svg = kit.Svg(900, 416, "Cortex calibration: CPU to input current to firing rate",
                  "Input current and resulting firing rate against CPU for the four neuron types.")
    svg.title_text("cortex (planned): CPU -> input current -> firing rate, simulated")
    cpus = [0.3 * (1000.0 / 0.3) ** (i / 40.0) for i in range(41)]  # percent, 0.3 to 1000
    cpus = [c for c in cpus if c <= 300.0]
    left = kit.Axes(svg, 70, 60, 360, 280, (0.3, 300), (0, 52), xlog=True,
                    xlabel="CPU (percent of one core)", ylabel="input current I",
                    title="current map (piecewise linear in cpu)")
    right = kit.Axes(svg, 520, 60, 350, 280, (0.3, 300), (0, 120), xlog=True,
                     xlabel="CPU (percent of one core)", ylabel="firing rate (Hz)",
                     title="resulting rate, noise sigma 1.2")
    right.hline(40, color=kit.MUTED)
    right.label(0.35, 40, "40 Hz", color=kit.MUTED, size=10, dy=-5)
    right.vline(5, color=kit.MUTED)
    right.vline(100, color=kit.MUTED)
    entries = []
    for kind, label, colour in KINDS:
        rh, i40 = izh.calibrate(kind, step=0.05)
        currents = [current(rh, i40, c / 100.0) for c in cpus]
        rates = [izh.rate_hz(kind, i, noise=1.2, seed=7, settle_ms=500.0, window_ms=4000.0)
                 for i in currents]
        left.line(cpus, currents, color=colour, width=1.8)
        right.line(cpus, rates, color=colour, width=1.8)
        right.points(cpus, rates, color=colour, r=1.8)
        entries.append(("%s (%s)" % (kind, label), colour, None))
        print(kind, label, "rheobase %.2f  I40 %.2f" % (rh, i40))
        for c in (0.5, 2.0, 5.0, 10.0, 50.0, 100.0, 300.0):
            i = current(rh, i40, c / 100.0)
            print("   cpu %5.1f %%  I %5.2f  rate %6.2f Hz" % (
                c, i, izh.rate_hz(kind, i, noise=1.2, seed=7, settle_ms=500.0, window_ms=4000.0)))
    left.legend(entries, corner="tl")
    svg.text(450, 404, "silent up to about 3 % CPU; at 5 % the noisy neurons already fire 6 to 20 Hz; "
             "one full core gives 39 to 44 Hz for every type", size=11, anchor="middle", fill=kit.MUTED)
    svg.save("cortex-calibration")


if __name__ == "__main__":
    main()
