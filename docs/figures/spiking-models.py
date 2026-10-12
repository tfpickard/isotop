"""Hodgkin-Huxley and four Izhikevich firing classes, simulated in pure Python.

Run from the repository root: python3 docs/figures/spiking-models.py
Writes docs/media/figures/spiking-models.svg.

Panel A: Hodgkin-Huxley (modern convention, rest -65 mV), a 10 uA/cm^2 step at t = 5 ms.
Panels B to E: Izhikevich (2003) regular spiking, intrinsically bursting, chattering and fast
spiking for a step of I = 10 at t = 50 ms, Euler steps of 0.5 ms. Panel F: noise-free f-I curves of
the four Izhikevich types, rate over 2 s after 1 s of settling.
"""

import hh_w8 as hh
import izh_w8 as izh
import svgkit_w8 as kit

TYPE_COLOURS = {"RS": kit.AQUA, "IB": kit.VIOLET, "CH": kit.YELLOW, "FS": kit.ORANGE}
TYPE_NAMES = {"RS": "regular spiking (a, b, c, d = 0.02, 0.2, -65, 8)",
              "IB": "intrinsically bursting (0.02, 0.2, -55, 4)",
              "CH": "chattering (0.02, 0.2, -50, 2)",
              "FS": "fast spiking (0.1, 0.2, -65, 2)"}


def main():
    svg = kit.Svg(960, 640, "Spiking neuron models: Hodgkin-Huxley and Izhikevich",
                  "Membrane traces of the Hodgkin-Huxley model and four Izhikevich firing classes, "
                  "and their f-I curves.")
    svg.title_text("Spiking models: one conductance model, four two-variable classes")
    w, h = 270, 150
    xs = [58, 368, 678]
    ys = [64, 360]
    # A: Hodgkin-Huxley
    t, v, n, m, hg = hh.run(lambda s: 10.0 if s >= 5.0 else 0.0, 40.0, dt=0.02)
    ax = kit.Axes(svg, xs[0], ys[0], w, h - 35, (0, 40), (-85, 50), ylabel="V (mV)",
                  title="A  Hodgkin-Huxley, I = 10 uA/cm^2", xticks=[0, 10, 20, 30, 40],
                  yticks=[-80, -40, 0, 40])
    ax.line(t, v, color=kit.BLUE, width=1.8)
    bx = kit.Axes(svg, xs[0], ys[0] + h - 5, w, 52, (0, 40), (0, 1), xlabel="t (ms)",
                  xticks=[0, 10, 20, 30, 40], yticks=[0, 0.5, 1])
    bx.line(t, m, color=kit.ORANGE, width=1.4)
    bx.line(t, hg, color=kit.AQUA, width=1.4)
    bx.line(t, n, color=kit.VIOLET, width=1.4)
    svg.text(xs[0] + w - 4, ys[0] + h - 12, "m", size=10, anchor="end", fill=kit.ORANGE)
    svg.text(xs[0] + w - 18, ys[0] + h - 12, "h", size=10, anchor="end", fill=kit.AQUA)
    svg.text(xs[0] + w - 32, ys[0] + h - 12, "n", size=10, anchor="end", fill=kit.VIOLET)
    # B to E
    cells = [("RS", 1, 0), ("IB", 2, 0), ("CH", 0, 1), ("FS", 1, 1)]
    for kind, col, row in cells:
        spikes, trace = izh.simulate(kind, 0.0, 450.0, record=True,
                                     current_of_time=lambda s: 10.0 if s >= 50.0 else 0.0)
        ax = kit.Axes(svg, xs[col], ys[row], w, h - 10, (0, 450), (-85, 35),
                      xlabel="t (ms)" if row else "", ylabel="v (mV)" if col == 0 or row == 0 and col == 1 else "",
                      title="%s  %s" % ("BCDE"[[c[0] for c in cells].index(kind)], kind),
                      xticks=[0, 150, 300, 450], yticks=[-80, -40, 0, 30])
        ax.line([p[0] for p in trace], [p[1] for p in trace], color=TYPE_COLOURS[kind], width=1.4)
        isis = [b - a for a, b in zip(spikes, spikes[1:])]
        late = [s for s in spikes if s >= 50.0]
        ax.label(452, -80, TYPE_NAMES[kind].split(" (")[0] + ", %d spikes" % len(late), size=10,
                 anchor="end", color=kit.MUTED, dy=-4)
        print(kind, "spikes", len(late), "first ISIs", [round(x, 1) for x in isis[:5]],
              "last ISI %.1f" % (isis[-1] if isis else 0))
    # F: f-I
    ax = kit.Axes(svg, xs[2], ys[1], w, h - 10, (3, 20), (0, 250), xlabel="I", ylabel="rate (Hz)",
                  title="F  f-I curves (noise-free)", xticks=[4, 8, 12, 16, 20])
    currents = [3.0 + 0.25 * i for i in range(69)]
    entries = []
    for kind in ("RS", "IB", "CH", "FS"):
        rates = [izh.rate_hz(kind, i) for i in currents]
        ax.line(currents, rates, color=TYPE_COLOURS[kind], width=1.8)
        entries.append((kind, TYPE_COLOURS[kind], None))
    ax.legend(entries, corner="tl")
    svg.text(480, 624, "Hodgkin and Huxley's model needs four coupled equations and a dozen rate functions; "
             "Izhikevich's two-variable model reproduces the same firing classes at a fraction of the cost.",
             size=10.5, anchor="middle", fill=kit.MUTED)
    svg.save("spiking-models")


if __name__ == "__main__":
    main()
