"""The spectrum of one minute of a demo process, and what truncating it costs.

Run from the repository root: python3 docs/figures/fourier-epicycle-spectrum.py
Writes docs/media/figures/fourier-epicycle-spectrum.svg. Standard library only.

Left: |c_k| of the 60-sample window of demo process 12 (the same signal as the reconstruction
figure), by signed frequency in turns per window; terms the view would keep (the 12 largest) in
colour. Right: the rms error of the K-term chain, measured at the 60 samples, against Parseval's
prediction sqrt(sum of omitted |c_k|^2), and the share of the signal's power the K terms hold.
"""

import math
import os
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from epicycle_w6 import dft, epicycles, rms_error, window
from svgkit_w6 import AQUA, BLUE, MAGENTA, MUTED, ORANGE, TEXT, YELLOW, Axes, Svg, heading


def main():
    z, _ = window(12, 240.0)
    n = len(z)
    spectrum = [abs(c) / n for c in dft(z)]
    keep = [f for f, _ in epicycles(z, 12)]
    power = sum(abs(v) ** 2 for v in z) / n
    ranked = sorted(range(1, n), key=lambda k: (-spectrum[k], k))
    svg = Svg(1080, 430, "Spectrum and truncation error of an epicycle chain",
              "Amplitude spectrum of a 60-sample window and the measured and Parseval truncation "
              "error against the number of epicycles kept.")
    heading(svg, "Which epicycles to keep: the spectrum and the cost of truncating it",
            "demo process 12, window starting at t = 240 s; frequencies are turns per 60 samples")
    left = Axes(svg, 64, 90, 470, 260, (-31, 30), (0, 0.22), "frequency k (turns per window)",
                "|c_k|  (cores or doublings)", "Amplitude spectrum, mean removed",
                xticks=[-30, -20, -10, 0, 10, 20, 29], yticks=[0, 0.05, 0.1, 0.15, 0.2])
    for k in range(1, n):
        f = k if k < n // 2 else k - n
        color = ORANGE if f in keep else MUTED
        left.svg.line(left.px(f), left.py(0), left.px(f), left.py(spectrum[k]), color, 2.4)
        left.svg.circle(left.px(f), left.py(spectrum[k]), 2.8, color)
    left.legend([("12 largest: the chain isotop draws", ORANGE, "dot"),
                 ("the other 47, dropped", MUTED, "dot")], x=left.x + 12, y=left.y + 44,
                size=10.5)
    left.label(-30.5, 0.208, "tall stems: k = +/-1 (memory swing) and +/-5 (5.25 CPU pulses per window)",
               10, "start", TEXT, dx=4)

    right = Axes(svg, 620, 90, 410, 260, (0, 59), (0, 1.0), "terms kept K",
                 "rms error  /  power captured", "Truncation",
                 xticks=[0, 8, 16, 24, 32, 40, 48, 59], yticks=[0, 0.2, 0.4, 0.6, 0.8, 1.0])
    ks = list(range(0, 60))
    measured, predicted, captured = [], [], []
    for k in ks:
        terms = epicycles(z, k) if k else []
        omitted = sum(spectrum[j] ** 2 for j in ranked[k:])
        predicted.append(math.sqrt(omitted))
        captured.append(1 - omitted / power)
        measured.append(rms_error(z, terms) if k else math.sqrt(power))
    right.line(ks, captured, AQUA, 2.2)
    right.line(ks, [m / measured[0] for m in measured], BLUE, 2.2)
    right.points(ks[::3], [p / measured[0] for p in predicted[::3]], YELLOW, 2.6)
    right.vline(12, MAGENTA, 1, "3 3")
    right.label(12, 0.08, "K = 12", 10, "start", MAGENTA, dx=4)
    right.legend([("share of the signal's power held by K terms", AQUA, "line"),
                  ("measured rms error / rms of the signal", BLUE, "line"),
                  ("sqrt(sum of omitted |c|^2) / rms (Parseval)", YELLOW, "dot")],
                 x=right.x + 120, y=right.y + 150, size=10.5)
    svg.text(24, 418, "The measured error equals Parseval's prediction to rounding at every K, "
                      "because the DFT is an orthogonal change of basis (up to the 1/N).", 10.5,
             "start", MUTED)
    print(svg.save("fourier-epicycle-spectrum"))
    print("K=12 power captured %.3f, rms ratio %.3f" % (captured[12], measured[12] / measured[0]))


if __name__ == "__main__":
    main()
