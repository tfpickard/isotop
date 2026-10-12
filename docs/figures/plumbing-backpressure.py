"""Backpressure in a Unix pipeline, as a fluid model, and how fast a pipe fills.

Run from the repository root: python3 docs/figures/plumbing-backpressure.py
Writes docs/media/figures/plumbing-backpressure.svg.

Left: `cat access.log | grep 500 | sort | uniq -c` with 64 KiB pipes. cat can write 48 MB/s, grep
consumes at most 20 MB/s and passes 5 % of what it reads, sort reads at most 2 MB/s and (like the
real sort) writes nothing until its input ends, so uniq starves. Euler steps of 0.1 ms; a stage
blocks when its output pipe is full (pipe_write) or its input pipe is empty (pipe_read).
Right: the time a pipe of capacity C takes to fill when the writer outpaces the reader by D bytes
per second is C / D.
"""

import svgkit_w8 as kit

CAPACITY = 65536.0
DT = 1e-4


def simulate(duration=0.4):
    cat_rate, grep_rate, selectivity, sort_rate = 48e6, 20e6, 0.05, 2e6
    q1 = q2 = 0.0
    times, fill1, fill2, grep_in, cat_out = [], [], [], [], []
    steps = int(duration / DT)
    for s in range(steps + 1):
        t = s * DT
        written = min(cat_rate * DT, CAPACITY - q1)
        consumed = min(grep_rate * DT, q1)
        if q2 + selectivity * consumed > CAPACITY:
            consumed = (CAPACITY - q2) / selectivity
        sorted_in = min(sort_rate * DT, q2)
        q1 += written - consumed
        q2 += selectivity * consumed - sorted_in
        if s % 20 == 0:
            times.append(t * 1000.0)
            fill1.append(100.0 * q1 / CAPACITY)
            fill2.append(100.0 * q2 / CAPACITY)
            grep_in.append(consumed / DT / 1e6)
            cat_out.append(written / DT / 1e6)
    return times, fill1, fill2, grep_in, cat_out


def main():
    times, fill1, fill2, grep_in, cat_out = simulate()
    svg = kit.Svg(900, 416, "Backpressure in a pipeline and the time a pipe takes to fill",
                  "Fill levels of two pipes in a four-stage pipeline, and the time to fill a pipe "
                  "against writer-reader mismatch.")
    svg.title_text("A pipe is a 64 KiB bucket between two rates")
    ax = kit.Axes(svg, 70, 60, 370, 280, (0, 400), (0, 110), xlabel="time (ms)",
                  ylabel="pipe fill (% of 64 KiB)", title="cat | grep 500 | sort | uniq -c",
                  xticks=[0, 100, 200, 300, 400], yticks=[0, 25, 50, 75, 100])
    ax.line(times, fill1, color=kit.RED, width=2.2)
    ax.line(times, fill2, color=kit.AQUA, width=2.2)
    ax.hline(100, color=kit.MUTED)
    ax.legend([("cat -> grep: full, cat blocked in pipe_write", kit.RED, None),
               ("grep -> sort: empty, sort waits in pipe_read", kit.AQUA, None)], corner="br")
    t_full = next(t for t, f in zip(times, fill1) if f >= 95.0)
    print("pipe 1 above 95 %% after %.1f ms; steady grep input %.1f MB/s; pipe 2 fill %.2f %%" % (
        t_full, grep_in[-1], fill2[-1]))
    bx = kit.Axes(svg, 520, 60, 350, 280, (1e3, 1e8), (1e-3, 1e2), xlog=True, ylog=True,
                  xlabel="writer rate minus reader rate (bytes/s)", ylabel="time to fill (s)",
                  title="t = C / D")
    rates = [10 ** (3 + 5 * i / 100.0) for i in range(101)]
    entries = []
    for capacity, name, colour in ((4096.0, "4 KiB (pre-2.6.11 pipe)", kit.YELLOW),
                                   (65536.0, "64 KiB (Linux default)", kit.BLUE),
                                   (1048576.0, "1 MiB (pipe-max-size default)", kit.VIOLET)):
        bx.line(rates, [capacity / d for d in rates], color=colour, width=2.0)
        entries.append((name, colour, None))
    bx.hline(0.1, color=kit.MUTED)
    bx.label(1.2e3, 0.1, "0.1 s", color=kit.MUTED, size=10, dy=-4)
    bx.legend(entries, corner="tr")
    svg.text(450, 406, "the full pipe sits just upstream of the slowest stage; empty pipes sit "
             "downstream of it", size=11, anchor="middle", fill=kit.MUTED)
    svg.save("plumbing-backpressure")


if __name__ == "__main__":
    main()
