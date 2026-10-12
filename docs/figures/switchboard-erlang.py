"""Erlang B and Erlang C: the honest mathematics of a switchboard.

Run from the repository root: python3 docs/figures/switchboard-erlang.py
Writes docs/media/figures/switchboard-erlang.svg.

Left: Erlang B, the blocking probability of N trunks offered A erlangs when blocked calls are
cleared. Right: Erlang C, the probability that an arrival has to wait for one of N operators at
utilisation rho = A / N when waiting calls queue (M/M/N). Both use the exact recurrences.
"""

import svgkit_w8 as kit


def erlang_b(servers, load):
    b = 1.0
    for k in range(1, servers + 1):
        b = load * b / (k + load * b)
    return b


def erlang_c(servers, load):
    if load >= servers:
        return 1.0
    b = erlang_b(servers, load)
    return servers * b / (servers - load * (1.0 - b))


def main():
    svg = kit.Svg(900, 420, "Erlang B and Erlang C",
                  "Blocking probability against offered load for several trunk counts, and the "
                  "probability of waiting against utilisation for several operator counts.")
    svg.title_text("Switchboard queueing: loss (Erlang B) and delay (Erlang C)")
    colours = [kit.RED, kit.ORANGE, kit.YELLOW, kit.AQUA, kit.BLUE, kit.VIOLET]
    left = kit.Axes(svg, 70, 62, 360, 290, (0.1, 100), (1e-4, 1), xlog=True, ylog=True,
                    xlabel="offered load A (erlangs)", ylabel="blocking probability B(N, A)",
                    title="Erlang B: calls cleared")
    loads = [0.1 * (1000.0) ** (i / 200.0) for i in range(201)]
    entries = []
    for servers, colour in zip((1, 2, 5, 10, 20, 50), colours):
        left.line(loads, [erlang_b(servers, a) for a in loads], color=colour, width=1.9)
        entries.append(("N = %d" % servers, colour, None))
    left.hline(0.01, color=kit.MUTED)
    left.label(0.12, 0.01, "1 %", color=kit.MUTED, size=10, dy=-4)
    left.legend(entries, corner="br")
    right = kit.Axes(svg, 520, 62, 350, 290, (0, 1), (0, 1), xlabel="utilisation rho = A / N",
                     ylabel="probability of waiting C(N, A)", title="Erlang C: calls queue",
                     xticks=[0, 0.25, 0.5, 0.75, 1.0])
    rhos = [i / 400.0 for i in range(0, 400)]
    entries = []
    for servers, colour in zip((1, 2, 4, 8, 16, 64), colours):
        right.line(rhos, [erlang_c(servers, rho * servers) for rho in rhos], color=colour, width=1.9)
        entries.append(("N = %d" % servers, colour, None))
    right.legend(entries, corner="tl")
    svg.text(450, 404, "at the same utilisation, a bigger pool waits less: pooling is why one operator "
             "pool serves many lines", size=11, anchor="middle", fill=kit.MUTED)
    svg.save("switchboard-erlang")


if __name__ == "__main__":
    main()
