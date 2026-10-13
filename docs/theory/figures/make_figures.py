#!/usr/bin/env python3
"""Regenerates the diagrams in docs/theory/*.md.

Needs matplotlib and numpy (not part of isotop; the mockup scripts already use numpy):

    python3 -m venv /tmp/figenv && /tmp/figenv/bin/pip install matplotlib numpy
    /tmp/figenv/bin/python docs/theory/figures/make_figures.py

Every constant is copied from the source file named in the comment beside it, and the glyph is
read from src/glyphs.rs, so a figure that disagrees with the code is a bug in this file.
"""
import math
import os
import pathlib
import re

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

HERE = pathlib.Path(__file__).resolve().parent
SRC = HERE.parents[2] / "src"

INK, GRID = "#222222", "#cfcfcf"
BLUE, ORANGE, GREEN, RED, PURPLE, GREY = "#2f6fb3", "#d8741b", "#3c9d5d", "#c0392b", "#7d4fa3", "#8a8a8a"
plt.rcParams.update(
    {
        "svg.fonttype": "none",  # keep text as text: small files, selectable
        "font.family": "sans-serif",
        "font.size": 9,
        "axes.edgecolor": INK,
        "axes.labelcolor": INK,
        "xtick.color": INK,
        "ytick.color": INK,
        "text.color": INK,
        "axes.grid": True,
        "grid.color": GRID,
        "grid.linewidth": 0.5,
        "figure.facecolor": "white",
        "axes.facecolor": "white",
        "savefig.facecolor": "white",
        "axes.spines.top": False,
        "axes.spines.right": False,
    }
)


def save(figure, name):
    figure.savefig(HERE / f"{name}.svg", format="svg", metadata={"Date": None}, bbox_inches="tight")
    if os.environ.get("FIGURE_PREVIEW"):  # a PNG per figure in that directory, for a quick look
        figure.savefig(pathlib.Path(os.environ["FIGURE_PREVIEW"]) / f"{name}.png", dpi=110, bbox_inches="tight")
    plt.close(figure)


# ---------------------------------------------------------------- Horizon
def photon_path(impact):
    """Backward-traced photon from the observer at +x: u = 1/r against the sweep phi,
    d2u/dphi2 = 3u^2 - u (src/horizon/lens.rs, trace_photon), by RK4."""
    h = 0.002
    u, v, phi = 0.0, 1.0 / impact, 0.0
    points = []
    for _ in range(int(3 * math.tau / h)):
        if u > 0.5 or (u < -0.0 and phi > 0.1):
            break
        points.append((u, phi))
        f = lambda u_: 3 * u_ * u_ - u_
        k1u, k1v = v, f(u)
        k2u, k2v = v + 0.5 * h * k1v, f(u + 0.5 * h * k1u)
        k3u, k3v = v + 0.5 * h * k2v, f(u + 0.5 * h * k2u)
        k4u, k4v = v + h * k3v, f(u + h * k3u)
        u += h / 6 * (k1u + 2 * k2u + 2 * k3u + k4u)
        v += h / 6 * (k1v + 2 * k2v + 2 * k3v + k4v)
        phi += h
    return points


def horizon_rays():
    fig, ax = plt.subplots(figsize=(7.2, 4.2))
    critical = 3 * math.sqrt(3)
    for impact in [1.5, 3.0, 4.2, 5.0, 5.15, 5.25, 5.5, 6.5, 8.0, 10.0]:
        pts = photon_path(impact)
        r = np.array([1 / max(u, 1e-4) for u, _ in pts])
        phi = np.array([p for _, p in pts])
        keep = r < 60
        captured = impact < critical
        ax.plot(r[keep] * np.cos(phi[keep]), r[keep] * np.sin(phi[keep]),
                color=RED if captured else BLUE, lw=1.0, alpha=0.9)
    t = np.linspace(0, math.tau, 400)
    ax.fill(2 * np.cos(t), 2 * np.sin(t), color="black", zorder=3)
    for radius, style, color, label in [(3, "-", ORANGE, "photon sphere r = 3"), (6, "--", GREY, "ISCO r = 6"),
                                        (critical, ":", INK, "shadow edge b = 3√3")]:
        ax.plot(radius * np.cos(t), radius * np.sin(t), style, color=color, lw=1.0, label=label, zorder=2)
    ax.set_xlim(-12, 24)
    ax.set_ylim(-11, 11)
    ax.set_aspect("equal")
    ax.set_xlabel("x / M   (observer far away to the right)")
    ax.set_ylabel("y / M")
    ax.set_title("Light rays traced back from the observer: red falls in (b < 5.196), blue escapes", fontsize=9)
    ax.legend(loc="upper left", fontsize=8, frameon=False)
    save(fig, "horizon-rays")


def horizon_potential():
    fig, ax = plt.subplots(figsize=(6.4, 3.4))
    r = np.linspace(2.05, 40, 800)
    for l2, color, label in [(10, GREY, "L² = 10: no circular orbit"),
                             (12, RED, "L² = 12: ISCO (inflection at r = 6)"),
                             (16, BLUE, "L² = 16: stable minimum, unstable maximum")]:
        ax.plot(r, (1 - 2 / r) * (1 + l2 / r**2), color=color, lw=1.4, label=label)
    ax.axvline(6, color=INK, lw=0.6, ls="--")
    ax.set_xlim(2, 30)
    ax.set_ylim(0.85, 1.05)
    ax.set_xlabel("r / M")
    ax.set_ylabel("effective potential V  (= E²)")
    ax.set_title("A circular orbit sits where V' = 0; it is stable only at a minimum", fontsize=9)
    ax.legend(fontsize=7.5, frameon=False, loc="upper right")
    save(fig, "horizon-potential")


# ---------------------------------------------------------------- Cortex
PATTERNS = {  # src/neurons.rs, Firing::params
    "FS fast spiking (kernel)": (0.1, 0.2, -65.0, 2.0),
    "RS regular spiking (system)": (0.02, 0.2, -65.0, 8.0),
    "CH chattering (session)": (0.02, 0.2, -50.0, 2.0),
    "IB intrinsically bursting (container)": (0.02, 0.2, -55.0, 4.0),
}


def izhikevich(params, current, milliseconds, record=False):
    """src/neurons.rs, advance: one Euler step of 0.5 ms; fires when v reached 30 mV."""
    a, b, c, d = params
    v, u, spikes, trace = -65.0, b * -65.0, [], []
    for step in range(int(milliseconds / 0.5)):
        if v >= 30:
            v, u = c, u + d
            spikes.append(step * 0.5)
        v += 0.5 * (0.04 * v * v + 5 * v + 140 - u + current)
        u += 0.5 * a * (b * v - u)
        v = min(v, 30.0)
        trace.append(v)
    return (spikes, trace) if record else spikes


def cortex_patterns():
    fig, axes = plt.subplots(4, 1, figsize=(7.0, 6.0), sharex=True)
    for ax, (name, params) in zip(axes, PATTERNS.items()):
        _, trace = izhikevich(params, 10.0, 400, record=True)
        ax.plot(np.arange(len(trace)) * 0.5, trace, color=BLUE, lw=0.8)
        ax.set_ylabel("v (mV)")
        ax.set_title(name, loc="left", fontsize=8, pad=2)
        ax.set_ylim(-75, 35)
    axes[-1].set_xlabel("time (ms), constant input current I = 10, no noise")
    fig.suptitle("The four firing patterns the process kinds map to", fontsize=9, y=0.995)
    fig.tight_layout()
    save(fig, "cortex-patterns")


def cortex_fi():
    fig, ax = plt.subplots(figsize=(6.4, 3.4))
    currents = np.concatenate([np.arange(3.5, 4.6, 0.05), np.arange(5.0, 21.0, 0.5)])
    for (name, params), color in zip(PATTERNS.items(), [GREEN, BLUE, ORANGE, PURPLE]):
        rates = []
        for current in currents:
            spikes = izhikevich(params, current, 3000)
            rates.append(sum(1 for s in spikes if s >= 1000) / 2.0)
        ax.plot(currents, rates, color=color, lw=1.4, label=name)
    ax.axvline(4.0, color=INK, lw=0.6, ls="--")
    ax.text(4.2, 95, "I = 4: rest and\nthreshold collide", fontsize=7.5)
    ax.set_xlabel("input current I")
    ax.set_ylabel("steady rate (spikes/s)")
    ax.set_ylim(0, 230)
    ax.set_title("Rate against current: it climbs continuously from zero, steeply at first", fontsize=9)
    ax.legend(fontsize=7.5, frameon=False, loc="upper left", bbox_to_anchor=(0.02, 0.98))
    save(fig, "cortex-rate-current")


# ---------------------------------------------------------------- Ptolemy
def ptolemy_signal():
    n = np.arange(60)
    cores = 0.8 + 0.6 * np.sin(2 * np.pi * n / 60 * 2) + 0.25 * np.sin(2 * np.pi * n / 60 * 7 + 1) \
        + 0.35 * (n % 20 > 14)
    doublings = 7.0 + 0.9 * np.sin(2 * np.pi * n / 60 * 1 + 0.5) + 0.3 * np.cos(2 * np.pi * n / 60 * 5)
    z = cores + 1j * doublings
    return z - z.mean()


def ptolemy_epicycles():
    z = ptolemy_signal()
    spectrum = np.fft.fft(z)  # forward sign exp(-2 pi i k n / N), as simulation::dft
    order = sorted(range(1, 60), key=lambda k: -abs(spectrum[k]))
    n = np.arange(60)
    fig, axes = plt.subplots(1, 4, figsize=(8.4, 2.6), sharex=True, sharey=True)
    for ax, keep in zip(axes, [None, 2, 4, 12]):
        if keep is None:
            w, title = z, "the minute: 60 samples"
        else:
            w = sum(spectrum[k] / 60 * np.exp(2j * np.pi * k * n / 60) for k in order[:keep])
            title = f"{keep} epicycles" if keep != 2 else "2 epicycles (a counter-rotating pair is a line)"
        ax.plot(w.real, w.imag, "-o", color=BLUE if keep else INK, ms=2, lw=0.8)
        ax.set_title(title, fontsize=8)
        ax.set_aspect("equal")
        ax.set_xticks([])
        ax.set_yticks([])
    axes[0].set_ylabel("log₂ memory")
    fig.supxlabel("CPU (cores)   —   the path of one process through (CPU, memory), mean removed", fontsize=8)
    save(fig, "ptolemy-epicycles")


def ptolemy_retrograde():
    fig, axes = plt.subplots(1, 2, figsize=(7.2, 3.3))
    for ax, (we, label) in zip(axes, [(1.5, "ρωe = 0.75 < Rωd = 1: no loops"),
                                      (4.0, "ρωe = 2 > Rωd = 1: retrograde loops")]):
        R, rho, wd = 1.0, 0.5, 1.0
        t = np.linspace(0, 4 * np.pi, 8000)
        p = R * np.exp(1j * wd * t) + rho * np.exp(1j * we * t)
        dp = 1j * wd * R * np.exp(1j * wd * t) + 1j * we * rho * np.exp(1j * we * t)
        back = (np.conj(p) * dp).imag < 0  # d(longitude)/dt < 0
        ax.plot(p.real, p.imag, color=BLUE, lw=1.0)
        ax.plot(np.where(back, p.real, np.nan), np.where(back, p.imag, np.nan), color=RED, lw=2.2)
        ax.plot(0, 0, "o", color=GREEN, ms=5)
        ax.text(0.04, 0.06, "Earth", color=GREEN, fontsize=8)
        ax.set_aspect("equal")
        ax.set_title(label, fontsize=8)
        ax.set_xticks([])
        ax.set_yticks([])
    fig.suptitle("A planet on a deferent (R = 1) with an epicycle (ρ = 0.5); red where Im(p̅ p') < 0", fontsize=9, y=1.02)
    save(fig, "ptolemy-retrograde")


# ---------------------------------------------------------------- Tesseract
def tesseract_boost():
    fig, axes = plt.subplots(1, 2, figsize=(7.4, 3.8), sharey=True)
    eta = math.atanh(0.5)
    w = np.linspace(-1, 1, 2)
    for ax, boosted in zip(axes, [False, True]):
        for beta, color, label in [(0.0, GREY, "at rest"), (0.5, BLUE, "0.5 core"), (0.9, GREEN, "0.9 core"),
                                   (1.0, ORANGE, "1 core (lightlike)"), (1.5, RED, "1.5 cores (superluminal)")]:
            x, ww = beta * w, w
            if boosted:
                x, ww = x * math.cosh(eta) - ww * math.sinh(eta), ww * math.cosh(eta) - x * math.sinh(eta)
            ax.plot(x, ww, color=color, lw=1.6, ls="--" if beta == 1.0 else "-", label=label)
        ax.set_xlim(-1.6, 1.6)
        ax.set_ylim(-1.1, 1.1)
        ax.set_aspect("equal")
        ax.set_xlabel("x  (CPU-seconds / 30)")
        ax.set_title("rest frame of the 0.5-core process" if boosted else "lab frame", fontsize=9)
    axes[0].set_ylabel("w  (time / 30 s)")
    handles, labels = axes[1].get_legend_handles_labels()
    fig.legend(handles, labels, fontsize=7.5, frameon=False, loc="lower center", ncol=5, bbox_to_anchor=(0.5, -0.04))
    fig.suptitle("A boost shears worldlines; slope 1 stays slope 1 and the process boosted to is vertical", fontsize=9, y=0.93)
    save(fig, "tesseract-boost")


def tesseract_schlegel():
    eye, world = 3.5, 25.0
    def project(x, y, z, w):
        factor = world / (eye - w)
        # oblique drawing of the 3D point (x, y, z)
        return factor * (x + 0.45 * y), factor * (z + 0.45 * y)
    fig, ax = plt.subplots(figsize=(4.6, 4.2))
    corners = [(a, b, c) for a in (-1, 1) for b in (-1, 1) for c in (-1, 1)]
    for w, color, label in [(1.0, BLUE, "w = +1  (now): near, large"), (-1.0, ORANGE, "w = −1  (a minute ago): far, small")]:
        pts = {c: project(*c, w) for c in corners}
        for a in corners:
            for b in corners:
                if sum(i != j for i, j in zip(a, b)) == 1 and a < b:
                    ax.plot(*zip(pts[a], pts[b]), color=color, lw=1.4)
        ax.plot([], [], color=color, label=label)
    for c in corners:
        ax.plot(*zip(project(*c, 1.0), project(*c, -1.0)), color=GREY, lw=0.7)
    ax.set_aspect("equal")
    ax.axis("off")
    ax.legend(fontsize=8, frameon=False, loc="lower center", bbox_to_anchor=(0.5, -0.12))
    ax.set_title("Schlegel projection, eye on the w axis at 3.5: the near cell is the outer cube", fontsize=8.5)
    save(fig, "tesseract-schlegel")


# ---------------------------------------------------------------- Coop
def coop_mappings():
    fig, axes = plt.subplots(1, 2, figsize=(7.4, 3.0))
    psi = np.linspace(0, 100, 300)
    for cv, color in [(0, BLUE), (1, GREEN), (2, RED)]:
        eta = np.minimum(0.5 + 0.5 * min(cv, 2) + 5.5 * psi / (psi + 15), 2 * math.pi)  # coop/sim.rs, noise
        axes[0].plot(psi, eta, color=color, label=f"cᵥ = {cv}")
    axes[0].axhline(2 * math.pi, color=INK, lw=0.6, ls="--")
    axes[0].set_xlabel("CPU pressure ψ (%)")
    axes[0].set_ylabel("heading noise η (rad)")
    axes[0].set_title("noise grows with pressure and burstiness", fontsize=8.5)
    axes[0].legend(fontsize=7.5, frameon=False)
    cpu = np.linspace(0, 400, 400)
    axes[1].plot(cpu, 0.6 + 3.4 * cpu / (cpu + 25), color=BLUE)  # coop/sim.rs, V_MIN, V_SPAN, SPEED_KNEE
    axes[1].set_xlabel("CPU (% of a core)")
    axes[1].set_ylabel("foraging speed (units/s)")
    axes[1].set_title("speed saturates at 4", fontsize=8.5)
    axes[1].set_ylim(0, 4.3)
    save(fig, "coop-mappings")


# ---------------------------------------------------------------- Matrix
def matrix_mechanics():
    text = (SRC / "glyphs.rs").read_text()
    values = [int(v, 16) for v in re.findall(r"0x([0-9a-f]+),", text)]
    bits = values[ord("A") - 0x20]
    fig, axes = plt.subplots(1, 3, figsize=(8.6, 3.2), gridspec_kw={"width_ratios": [1.1, 1, 1]})
    ax = axes[0]
    ax.set_xlim(0, 7)
    ax.set_ylim(14, 0)
    for r in range(14):
        for c in range(7):
            on = bits >> (r * 7 + c) & 1
            ax.add_patch(plt.Rectangle((c, r), 1, 1, fc=GREEN if on else "white", ec=GRID, lw=0.5))
            if r in (0, 1, 13) and c in (0, 1, 6):
                ax.text(c + 0.5, r + 0.5, str(r * 7 + c), ha="center", va="center", fontsize=6, color=INK)
    ax.set_aspect("equal")
    ax.axis("off")
    ax.set_title("'A' in 7×14: bit r·7 + c", fontsize=8.5)
    t = np.linspace(0, 3.5, 300)
    axes[1].plot(t, np.exp(-t / 0.35), color=GREEN)  # matrix.rs, FLASH
    axes[1].set_xlabel("seconds since struck")
    axes[1].set_ylabel("flash (1 = white)")
    axes[1].set_title("decode flash, τ = 0.35 s", fontsize=8.5)
    n = np.arange(0, 41)
    axes[2].plot(n, 0.5 / (1 + n / 3), color=BLUE)  # matrix.rs, advance
    axes[2].set_xlabel("lines waiting")
    axes[2].set_ylabel("seconds between lines")
    axes[2].set_title("release interval", fontsize=8.5)
    save(fig, "matrix-mechanics")


# ---------------------------------------------------------------- Necropolis
def necropolis_scales():
    fig, axes = plt.subplots(1, 3, figsize=(8.8, 2.9))
    ms = np.logspace(0, 7, 300)
    axes[0].semilogx(ms, np.minimum(0.55 + 0.36 * np.log10(np.maximum(ms, 3)), 3.2), color=BLUE)  # stone_height
    axes[0].set_xlabel("lifetime CPU (ms)")
    axes[0].set_ylabel("stone height")
    axes[0].set_title("height: 0.55 + 0.36 log₁₀(ms), ≤ 3.2", fontsize=8)
    mib = np.logspace(0, 4, 300)
    axes[1].semilogx(mib, np.minimum(0.62 + 0.15 * np.log2(np.maximum(mib, 1)), 2.4), color=ORANGE)  # stone_width
    axes[1].set_xlabel("peak memory (MiB)")
    axes[1].set_ylabel("stone width")
    axes[1].set_title("width: 0.62 + 0.15 log₂(MiB), ≤ 2.4", fontsize=8)
    age = np.linspace(0, 29.530588853, 300)
    axes[2].plot(age, (1 - np.cos(2 * np.pi * age / 29.530588853)) / 2, color=PURPLE)  # moon_lit
    axes[2].set_xlabel("moon age (days)")
    axes[2].set_ylabel("lit fraction")
    axes[2].set_title("moon: (1 − cos 2πa/P) / 2", fontsize=8)
    fig.tight_layout()
    save(fig, "necropolis-scales")


# ---------------------------------------------------------------- Arbor
def arbor_phyllotaxis():
    fig, axes = plt.subplots(1, 3, figsize=(8.4, 2.9))
    for ax, angle, label in zip(axes, [137.508, 137.0, 150.0],
                                ["137.508° (golden): even", "137.0°: spokes appear", "150.0°: rays"]):
        k = np.arange(1, 260)
        theta = np.radians(angle) * k
        r = np.sqrt(k)
        ax.scatter(r * np.cos(theta), r * np.sin(theta), s=6, color=GREEN, edgecolors="none")
        ax.set_aspect("equal")
        ax.set_title(label, fontsize=8.5)
        ax.axis("off")
    fig.suptitle("Placing each new organ at a fixed angle from the last (r ∝ √k)", fontsize=9)
    save(fig, "arbor-phyllotaxis")


def arbor_rules():
    fig, axes = plt.subplots(1, 2, figsize=(7.6, 3.1))
    ax = axes[0]
    ax.add_patch(plt.Circle((0, 0), 1.0, fc="#d9c2a3", ec=INK, lw=0.9))
    ax.text(0, -2.0, "parent\nr = 1", ha="center", fontsize=7.5)
    for cx, radius, label, color in [(3.2, 2 ** -0.5, "Leonardo\nr² adds: 0.707", BLUE),
                                     (6.4, 2 ** (-1 / 3), "Murray\nr³ adds: 0.794", ORANGE)]:
        for sign in (-1, 1):
            ax.add_patch(plt.Circle((cx + sign * radius, 0), radius, fc="#d9c2a3", ec=color, lw=1.4))
        ax.text(cx, -2.0, label, ha="center", fontsize=7.5, color=color)
    ax.set_xlim(-1.3, 8.0)
    ax.set_ylim(-2.7, 1.4)
    ax.set_aspect("equal")
    ax.axis("off")
    ax.set_title("daughters after an even split", fontsize=8.5)
    t = np.linspace(0, 6, 400)
    for zeta, color in [(0.12, RED), (0.5, ORANGE), (1.0, GREEN)]:
        omega = 2 * math.pi
        if zeta < 1:
            wd = omega * math.sqrt(1 - zeta**2)
            y = 1 - np.exp(-zeta * omega * t) * (np.cos(wd * t) + zeta / math.sqrt(1 - zeta**2) * np.sin(wd * t))
        else:
            y = 1 - (1 + omega * t) * np.exp(-omega * t)
        axes[1].plot(t, y, color=color, label=f"ζ = {zeta}" + ("  (arbor.rs)" if zeta == 0.12 else ""))
    axes[1].axhline(1, color=INK, lw=0.6, ls="--")
    axes[1].set_xlabel("time (natural periods)")
    axes[1].set_ylabel("branch lean / gust")
    axes[1].set_title("response to a step: ζ = 0.12 overshoots 68%", fontsize=8.5)
    axes[1].legend(fontsize=7.5, frameon=False, loc="lower right")
    save(fig, "arbor-rules")


if __name__ == "__main__":
    for make in [horizon_rays, horizon_potential, cortex_patterns, cortex_fi, ptolemy_epicycles, ptolemy_retrograde,
                 tesseract_boost, tesseract_schlegel, coop_mappings, matrix_mechanics, necropolis_scales,
                 arbor_phyllotaxis, arbor_rules]:
        make()
        print("wrote", make.__name__)
