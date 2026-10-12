"""Turing's diffusion-driven instability, worked on the Gray-Scott model.

Run from the repository root: python3 docs/figures/reaction-diffusion-dispersion.py
Writes docs/media/figures/reaction-diffusion-dispersion.svg.

At (F, k) = (0.0647, 0.0624) the non-trivial uniform state of Gray-Scott is stable without
diffusion (trace < 0, determinant > 0). With the diffusivity ratio Du/Dv that isotop uses (2) a
band of wavenumbers grows. The left panel is the growth rate of the fastest mode, Re lambda(q),
on the 5-point lattice; the right panel is Turing's determinant condition
h(q^2) = Du Dv q^4 - (Du g_v + Dv f_u) q^2 + det J, which must go negative for some q.
"""

import math

import rdsim_w8 as rd
import svgkit_w8 as kit

FEED, KILL = 0.0647, 0.0624


def critical_ratio(jac):
    """Smallest Du/Dv for which the continuum determinant condition is met."""
    fu, gv = jac[0][0], jac[1][1]
    det = fu * gv - jac[0][1] * jac[1][0]
    for i in range(1, 200000):
        d = i * 0.0005
        b = fu + d * gv
        if b > 0 and b * b > 4 * d * det:
            return d
    return None


def main():
    u, v = rd.steady_states(FEED, KILL)[1]
    jac = rd.jacobian(FEED, KILL, u, v)
    trace = jac[0][0] + jac[1][1]
    det = jac[0][0] * jac[1][1] - jac[0][1] * jac[1][0]
    print("u*=%.4f v*=%.4f" % (u, v))
    print("J =", jac)
    print("trace %.5f det %.6f" % (trace, det), "critical ratio", critical_ratio(jac))
    svg = kit.Svg(900, 400, "Diffusion-driven instability of the Gray-Scott uniform state",
                  "Growth rate against wavenumber for four diffusivity ratios, and Turing's "
                  "determinant condition.")
    svg.title_text("Turing instability at F = %g, k = %g (stable without diffusion)" % (FEED, KILL))
    qs = [math.pi * i / 400 for i in range(401)]
    ax = kit.Axes(svg, 70, 60, 360, 270, (0, 1.2), (-0.03, 0.045),
                  xlabel="wavenumber q (rad per cell)", ylabel="growth rate Re lambda per step",
                  title="fastest-growing mode on the lattice", xticks=[0, 0.3, 0.6, 0.9, 1.2],
                  yticks=[-0.02, 0, 0.02, 0.04])
    ratios = [(1.0, kit.MUTED, "Du/Dv = 1"), (1.5, kit.AQUA, "1.5"), (2.0, kit.YELLOW, "2 (isotop)"),
              (3.0, kit.ORANGE, "3")]
    for ratio, colour, name in ratios:
        rates = [rd.growth_rate(jac, q, du=ratio * rd.DV) for q in qs]
        ax.line(qs, rates, color=colour, width=2.0)
        best, where = rd.max_growth(jac, du=ratio * rd.DV, samples=400)
        print("ratio", ratio, "max growth %.5f at q = %.3f (wavelength %.1f cells)" % (
            best, where, 2 * math.pi / where if where > 0 else float("inf")))
    ax.hline(0, color=kit.TEXT, width=1)
    ax.legend([(name, colour, None) for _, colour, name in ratios], corner="tr")
    # unstable band for ratio 2
    low = [q for q in qs if rd.growth_rate(jac, q, du=2 * rd.DV) > 0]
    print("unstable band for ratio 2: q in [%.3f, %.3f]" % (low[0], low[-1]))
    ax.band(low[0], low[-1], color=kit.YELLOW, opacity=0.10)
    ax.label(0.62, -0.012, "stable at q = 0", dx=0, dy=0, color=kit.MUTED, size=10)
    # right panel: h(q^2)
    bx = kit.Axes(svg, 520, 60, 340, 270, (0, 1.2), (-0.0004, 0.0024),
                  xlabel="wavenumber q (continuum)", ylabel="h(q^2) = det(J - q^2 D)",
                  title="Turing's determinant condition", xticks=[0, 0.3, 0.6, 0.9, 1.2],
                  yticks=[0, 0.001, 0.002])
    qc = [1.2 * i / 400 for i in range(401)]
    for ratio, colour, name in ratios:
        du, dv = ratio * rd.DV, rd.DV
        b = du * jac[1][1] + dv * jac[0][0]
        h = [du * dv * q ** 4 - b * q * q + det for q in qc]
        bx.line(qc, h, color=colour, width=2.0)
    bx.hline(0, color=kit.TEXT, width=1)
    bx.label(0.06, 0.0017, "h > 0 everywhere: no band", color=kit.MUTED, size=10)
    bx.label(0.5, -0.0002, "h < 0: unstable", color=kit.YELLOW, size=10, dy=-4)
    svg.text(450, 380, "Du/Dv = 1 never goes unstable: the inhibitor must outrun the activator "
             "(here the substrate u, the faster species).", size=11, anchor="middle", fill=kit.MUTED)
    svg.save("reaction-diffusion-dispersion")


if __name__ == "__main__":
    main()
