"""Prototype of isotop's horizon view: the OOM killer as a Schwarzschild black hole.

Units G = c = M = 1. The camera is orthographic (observer at infinity) at inclination INC from
the disk normal. Every scene vertex (disk samples, process positions, orbit polylines, stars) is
mapped to its lensed image positions with a photon-orbit table built at startup by integrating
d2u/dphi2 = 3u^2 - u, then splatted. Nothing is ray traced per pixel.

usage: python3 -I horizon.py still|anim|test
"""
import math
import os
import sys
import time

PROTO = "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto"
sys.path.insert(0, PROTO)

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

import isostyle as S

BC = 3.0 * math.sqrt(3.0)          # critical impact parameter, shadow radius
R_IN, R_OUT = 6.0, 30.0            # disk: ISCO to 30M
INC = math.radians(80.0)           # camera inclination from face-on
STAR_SHELL = 40.0                  # background stars sit on a sphere this far behind the hole
M_PER_SECOND = 10.0                # animation: coordinate time per second of wall clock

# Observer direction and screen basis. Disk in the x-y plane, observer above it at -y.
O = np.array([0.0, -math.sin(INC), math.cos(INC)])
EX = np.array([1.0, 0.0, 0.0])
UP = np.array([0.0, math.cos(INC), math.sin(INC)])


# ----------------------------------------------------------------------------------------------
# Photon orbit table
# ----------------------------------------------------------------------------------------------
class LensTable:
    """b(r, sweep) for the primary (sweep psi) and secondary (sweep 2 pi - psi) images.

    Photons are traced back from the observer for a grid of impact parameters. For each target
    radius we record the sweep at which the photon crosses it inbound and, after periapsis,
    outbound. Along the path b: 0 -> b_max(r) inbound, then b_max(r) -> b_c outbound, the sweep
    rises monotonically from 0 to infinity, so it can be inverted by interpolation."""

    def __init__(self, n_r=360, n_psi=721, r_min=2.0005, r_max=150.0):
        t0 = time.time()
        below = BC * (1.0 - np.geomspace(1e-7, 1.0, 700)[::-1])
        below = below[below > 1e-4]
        above = BC + np.geomspace(1e-7, 80.0, 1100)
        b = np.concatenate([[1e-4], below, above])
        h = 0.002
        n_steps = int((2 * math.pi + 0.8) / h)
        u = np.zeros_like(b)
        v = 1.0 / b
        U = np.empty((n_steps + 1, b.size), np.float64)
        U[0] = u
        done = np.zeros(b.size, bool)
        for k in range(n_steps):
            def acc(x):
                return 3.0 * x * x - x
            k1u, k1v = v, acc(u)
            k2u, k2v = v + 0.5 * h * k1v, acc(u + 0.5 * h * k1u)
            k3u, k3v = v + 0.5 * h * k2v, acc(u + 0.5 * h * k2u)
            k4u, k4v = v + h * k3v, acc(u + h * k3u)
            un = u + h / 6 * (k1u + 2 * k2u + 2 * k3u + k4u)
            vn = v + h / 6 * (k1v + 2 * k2v + 2 * k3v + k4v)
            # absorbed (inside the horizon) or escaped back to infinity: freeze
            absorbed = un >= 0.6
            escaped = un <= -0.02
            un = np.where(done, u, np.clip(un, -0.02, 0.6))
            vn = np.where(done | absorbed | escaped, 0.0, vn)
            done |= absorbed | escaped
            u, v = un, vn
            U[k + 1] = u
        phi = np.arange(n_steps + 1) * h
        self.b_grid = b

        self.log_r = np.linspace(math.log(r_min), math.log(r_max), n_r)
        r_grid = np.exp(self.log_r)
        ug = 1.0 / r_grid
        phi_in = np.full((b.size, n_r), np.nan)
        phi_out = np.full((b.size, n_r), np.nan)
        for j in range(b.size):
            col = U[:, j]
            kmax = int(np.argmax(col))
            up = col[kmax]
            inc = col[: kmax + 1]
            phi_in[j] = np.interp(ug, inc, phi[: kmax + 1], right=np.nan)
            phi_in[j, ug > up] = np.nan
            if b[j] > BC and up < 0.5 and kmax < n_steps:
                dec = col[kmax:][::-1]
                pd = phi[kmax:][::-1]
                ok = ug >= dec[0]
                phi_out[j] = np.where(ok & (ug <= up), np.interp(ug, dec, pd), np.nan)
        self.psi = np.linspace(0.0, math.pi, n_psi)
        self.B1 = np.empty((n_r, n_psi))
        self.B2 = np.empty((n_r, n_psi))
        for i in range(n_r):
            s_in, b_in = phi_in[:, i], b
            ok = ~np.isnan(s_in)
            s_out = phi_out[::-1, i]
            b_out = b[::-1]
            ok2 = ~np.isnan(s_out)
            sweep = np.concatenate([[0.0], s_in[ok], s_out[ok2]])
            bb = np.concatenate([[0.0], b_in[ok], b_out[ok2]])
            sweep = np.maximum.accumulate(sweep)
            self.B1[i] = np.interp(self.psi, sweep, bb)
            self.B2[i] = np.interp(2 * math.pi - self.psi, sweep, bb)
        self.build_time = time.time() - t0

    def lookup(self, r, psi, table):
        lr = (np.log(r) - self.log_r[0]) / (self.log_r[1] - self.log_r[0])
        lr = np.clip(lr, 0, self.log_r.size - 1.001)
        lp = np.clip(psi / (self.psi[1] - self.psi[0]), 0, self.psi.size - 1.001)
        i0 = lr.astype(np.int32)
        j0 = lp.astype(np.int32)
        fr = (lr - i0).astype(np.float32)
        fp = (lp - j0).astype(np.float32)
        t = table
        return ((t[i0, j0] * (1 - fp) + t[i0, j0 + 1] * fp) * (1 - fr)
                + (t[i0 + 1, j0] * (1 - fp) + t[i0 + 1, j0 + 1] * fp) * fr)

    def images(self, P):
        """Lensed screen positions (in M, y up) of 3D points P[..., 3].
        Returns (x1, y1, x2, y2): primary and secondary image."""
        r = np.linalg.norm(P, axis=-1)
        n = P / r[..., None]
        c = np.clip(n @ O, -1.0, 1.0)
        psi = np.arccos(c)
        ex, ey = n @ EX, n @ UP
        s = np.hypot(ex, ey)
        s = np.where(s < 1e-9, 1e-9, s)
        ex, ey = ex / s, ey / s
        b1 = self.lookup(r, psi, self.B1)
        b2 = self.lookup(r, psi, self.B2)
        return b1 * ex, b1 * ey, -b2 * ex, -b2 * ey


def beloborodov_b(r, psi):
    cos_a = 1.0 - (1.0 - np.cos(psi)) * (1.0 - 2.0 / r)
    sin_a = np.sqrt(np.clip(1.0 - cos_a * cos_a, 0, None))
    return r * sin_a / np.sqrt(1.0 - 2.0 / r)


# ----------------------------------------------------------------------------------------------
# Disk physics and colour
# ----------------------------------------------------------------------------------------------
def page_thorne(r):
    """Novikov-Thorne / Page-Thorne flux for Schwarzschild (Luminet 1979 eq. 15), arbitrary units."""
    r = np.asarray(r, np.float64)
    s3, s6 = math.sqrt(3.0), math.sqrt(6.0)
    sr = np.sqrt(r)
    arg = (sr + s3) * (s6 - s3) / ((sr - s3) * (s6 + s3))
    f = (sr - s6 + (s3 / 3.0) * np.log(arg)) / ((r - 3.0) * r ** 2.5)
    return np.where(r > 6.0, f, 0.0)


def one_plus_z(r, x_screen):
    """Luminet 1979: (1 - 3/r)^-1/2 (1 + sqrt(1/r^3) b sin i sin alpha), with b sin alpha the
    image's screen x. Derived from L_z/E = -x sin i: the left side approaches."""
    return (1.0 - 3.0 / r) ** -0.5 * (1.0 + np.sqrt(1.0 / r ** 3) * math.sin(INC) * x_screen)


def _cie_lobe(lam, mu, s1, s2):
    s = np.where(lam < mu, s1, s2)
    return np.exp(-0.5 * ((lam - mu) / s) ** 2)


def blackbody_table(t_min=1000.0, t_max=40000.0, n=400):
    """B5: CIE 1931 2-degree CMFs (Wyman, Sloan, Shirley 2013 fit) against Planck, to linear
    sRGB normalized to the brightest channel."""
    lam = np.arange(380.0, 781.0, 2.0)
    xb = (1.056 * _cie_lobe(lam, 599.8, 37.9, 31.0) + 0.362 * _cie_lobe(lam, 442.0, 16.0, 26.7)
          - 0.065 * _cie_lobe(lam, 501.1, 20.4, 26.2))
    yb = 0.821 * _cie_lobe(lam, 568.8, 46.9, 40.5) + 0.286 * _cie_lobe(lam, 530.9, 16.3, 31.1)
    zb = 1.217 * _cie_lobe(lam, 437.0, 11.8, 36.0) + 0.681 * _cie_lobe(lam, 459.0, 26.0, 13.8)
    temps = np.geomspace(t_min, t_max, n)
    planck = lam[None, :] ** -5 / (np.exp(1.4388e7 / (lam[None, :] * temps[:, None])) - 1.0)
    X, Y, Z = planck @ xb, planck @ yb, planck @ zb
    rgb = np.stack([3.2406 * X - 1.5372 * Y - 0.4986 * Z,
                    -0.9689 * X + 1.8758 * Y + 0.0415 * Z,
                    0.0557 * X - 0.2040 * Y + 1.0570 * Z], -1)
    rgb = np.clip(rgb, 0, None)
    rgb /= rgb.max(axis=1, keepdims=True)
    return temps, rgb


BB_T, BB_RGB = blackbody_table()


def bb_colour(T):
    T = np.clip(T, BB_T[0], BB_T[-1])
    lt = np.log(T)
    return np.stack([np.interp(lt, np.log(BB_T), BB_RGB[:, c]) for c in range(3)], -1)


# ----------------------------------------------------------------------------------------------
# Plunge geodesic
# ----------------------------------------------------------------------------------------------
def plunge_track(l_frac=0.95, kick=0.1, t_max=400.0):
    """Coordinate-time track (t, r, dphi) of a body leaving the ISCO with angular momentum
    l_frac * L_isco and an inward kick dr/dtau = -kick. Integrated in proper time, recorded
    against coordinate time t. r tends to 2M but never reaches it."""
    L = l_frac * math.sqrt(12.0)
    r = 6.0
    E = math.sqrt((1 - 2 / r) * (1 + L * L / (r * r)) + kick * kick)
    tau_h = 0.01
    t, phi = 0.0, 0.0
    ts, rs, ps = [0.0], [r], [0.0]
    while t < t_max:
        # dr/dtau = -sqrt(E^2 - V), small kick to leave the turning point
        def drdtau(rr):
            v = E * E - (1 - 2 / rr) * (1 + L * L / (rr * rr))
            return -math.sqrt(max(v, 1e-12))
        k1 = drdtau(r)
        k2 = drdtau(r + 0.5 * tau_h * k1)
        k3 = drdtau(r + 0.5 * tau_h * k2)
        k4 = drdtau(r + tau_h * k3)
        rn = r + tau_h / 6 * (k1 + 2 * k2 + 2 * k3 + k4)
        rm = 0.5 * (r + rn)
        t += tau_h * E / (1 - 2 / rm)
        phi += tau_h * L / (rm * rm)
        r = rn
        if r < 2.0 + 1e-6:
            break
        ts.append(t)
        rs.append(r)
        ps.append(phi)
        # proper time step shrinks as we approach the horizon so t keeps advancing smoothly
        tau_h = min(0.01, 0.02 * (r - 2.0))
    ts, rs, ps = np.array(ts), np.array(rs), np.array(ps)
    return ts, rs, ps


# ----------------------------------------------------------------------------------------------
# Processes (demo data)
# ----------------------------------------------------------------------------------------------
NAMED = [
    # name, kind, oom_score, rss MiB, oom_score_adj
    ("worker-134", "container", 912, 3400, 0),
    ("browser-512", "session", 887, 2900, 0),
    ("compiler-90", "session", 803, 2100, 0),
    ("language-server-71", "session", 690, 1500, 0),
    ("postgres-41", "system", 655, 1300, 0),
    ("database-141", "container", 590, 1100, 0),
    ("renderer-52", "session", 561, 900, 0),
    ("redis-207", "container", 512, 700, 0),
    ("gnome-shell-88", "session", 470, 640, 0),
    ("tracker-191", "session", 402, 300, 0),
    ("nginx-128", "container", 371, 220, 0),
    ("pipewire-95", "session", 333, 120, 0),
    ("journald-310", "system", 61, 90, -250),
]
UNKILLABLE = [
    ("systemd-1", "system", 0, 60, -1000),
    ("sshd-702", "system", 0, 12, -1000),
    ("udevd-333", "system", 0, 18, -1000),
]
POOL = ["worker", "renderer", "shell", "dbus", "cron", "containerd", "evolution", "gvfsd",
        "ibus", "colord", "cupsd", "polkitd", "upowerd", "chronyd", "xdg-portal", "wireplumber",
        "node", "python", "java", "rustc", "cc1plus", "ld", "make", "bash", "zsh", "tmux",
        "vim", "ssh", "git", "agent", "exporter", "kubelet", "etcd", "grafana", "prometheus"]
N_PROCESSES = 192


def demo_processes():
    rng = np.random.default_rng(134)
    procs = [dict(name=n, kind=k, score=s, rss=m, adj=a) for n, k, s, m, a in NAMED]
    used = {p["name"] for p in procs}
    kinds = ["session"] * 4 + ["system"] * 3 + ["container"] * 3
    scores = np.sort(rng.integers(2, 330, N_PROCESSES - len(NAMED) - len(UNKILLABLE)))[::-1]
    for s in scores:
        while True:
            name = "%s-%d" % (POOL[rng.integers(len(POOL))], rng.integers(100, 999))
            if name not in used:
                break
        used.add(name)
        procs.append(dict(name=name, kind=kinds[rng.integers(len(kinds))], score=int(s),
                          rss=float(np.exp(rng.normal(3.2, 1.0))), adj=0))
    procs += [dict(name=n, kind=k, score=s, rss=m, adj=a) for n, k, s, m, a in UNKILLABLE]
    killable = sorted([p for p in procs if p["adj"] > -1000], key=lambda p: -p["score"])
    for rank, p in enumerate(killable, 1):
        p["rank"] = rank
    for p in procs:
        p["phi"] = float(rng.uniform(0, 2 * math.pi))
    return procs, len(killable)


N_ORBIT = 28          # the top N by oom_score ride the disk; the rest are counted, not drawn


def rank_radius(rank, n=N_ORBIT):
    """Rank 1 at the ISCO, growing outward by rank (rank based, so the kernel's scale is moot)."""
    return R_IN + 17.5 * ((rank - 1) / max(n - 1, 1)) ** 0.8


UNKILLABLE_R = 26.5


# ----------------------------------------------------------------------------------------------
# Rendering
# ----------------------------------------------------------------------------------------------
T0, T_GAMMA = 7200.0, 1.55        # colour temperature scale; exponent widens the ramp (stylized)
EXPOSURE = 0.36


def srgb(x):
    x = np.clip(x, 0, 1)
    return np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - 0.055)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


class Splatter:
    """Bilinear splat of points into an H x W canvas, with indices precomputed."""

    def __init__(self, X, Y, W, H):
        px = X.ravel() - 0.5
        py = Y.ravel() - 0.5
        i0 = np.floor(px).astype(np.int64)
        j0 = np.floor(py).astype(np.int64)
        fx = (px - i0).astype(np.float32)
        fy = (py - j0).astype(np.float32)
        idx, bw = [], []
        for di, dj, w in ((0, 0, (1 - fx) * (1 - fy)), (1, 0, fx * (1 - fy)),
                          (0, 1, (1 - fx) * fy), (1, 1, fx * fy)):
            ii, jj = i0 + di, j0 + dj
            ok = (ii >= 0) & (ii < W) & (jj >= 0) & (jj < H)
            idx.append(np.where(ok, jj * W + ii, H * W).astype(np.int32))
            bw.append(w)
        self.idx = np.concatenate(idx)
        self.bw = np.concatenate(bw)
        self.W, self.H = W, H

    def splat(self, weights):
        """weights: (N,) or (N, C). Returns (H, W) or (H, W, C)."""
        n = self.W * self.H
        if weights.ndim == 1:
            w = np.tile(weights.astype(np.float32), 4) * self.bw
            return np.bincount(self.idx, weights=w, minlength=n + 1)[:n].reshape(self.H, self.W)
        out = [self.splat(weights[:, c]) for c in range(weights.shape[1])]
        return np.stack(out, -1)


def warped_grid(need, lo, hi, target, n_min, n_max, periodic):
    """Samples on [lo, hi) whose spacing keeps need(x) * dx <= target pixels."""
    xs = need[0]
    dens = need[1] / target
    dens = ndimage.maximum_filter1d(dens, 9, mode="wrap" if periodic else "nearest")
    dens = ndimage.gaussian_filter1d(dens, 3, mode="wrap" if periodic else "nearest")
    dens = np.maximum(dens, 1e-3)
    cum = np.concatenate([[0], np.cumsum(0.5 * (dens[1:] + dens[:-1]) * np.diff(xs))])
    n = int(np.clip(cum[-1], n_min, n_max))
    u = (np.arange(n) + 0.5) / n * cum[-1]
    return np.interp(u, cum, xs)


class Scene:
    def __init__(self, W, H, scale, centre, lens, target_px=0.6, max_samples=9e6):
        t0 = time.time()
        self.W, self.H, self.s = W, H, scale
        self.cx, self.cy = centre
        self.lens = lens
        self.k = scale / 28.0          # size factor relative to the 1600 still
        # master emissivity texture in the co-rotating frame, uniform in (r, phi)
        self.tex_nr, self.tex_nphi = 960, 4096
        self.tex = self._texture(np.random.default_rng(5))
        self.layers = []
        for sign, table in ((1.0, lens.B1), (-1.0, lens.B2)):
            # coarse pre-pass to measure how many pixels a step in r and in phi covers
            rc = np.linspace(R_IN, R_OUT, 241)
            pc = np.linspace(0, 2 * math.pi, 4097)
            Xc, Yc = self._project(rc, pc, sign, table)
            dpp = np.hypot(np.diff(Xc, axis=1), np.diff(Yc, axis=1)) / np.diff(pc)[None, :]
            drr = np.hypot(np.diff(Xc, axis=0), np.diff(Yc, axis=0)) / np.diff(rc)[:, None]
            need_phi = (0.5 * (pc[1:] + pc[:-1]), dpp.max(axis=0))
            need_r = (0.5 * (rc[1:] + rc[:-1]), drr.max(axis=1))
            tgt = target_px
            while True:
                phi = warped_grid(need_phi, 0, 2 * math.pi, tgt, 512, 40000, True)
                r = warped_grid(need_r, R_IN, R_OUT, tgt, 64, 4000, False)
                if r.size * phi.size <= max_samples:
                    break
                tgt *= 1.1
            self.layers.append(self._build_layer(r, phi, sign, table))
            print("  layer %+d: %d x %d samples (%.2f px)" % (sign, r.size, phi.size, tgt),
                  flush=True)
        # Shadow mask and photon ring (drawn, not traced).
        yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
        dx, dy = xx + 0.5 - self.cx, yy + 0.5 - self.cy
        rho = np.hypot(dx, dy)
        self.shadow = np.clip(BC * scale - rho + 0.5, 0, 1)
        theta_x = dx / np.maximum(rho, 1e-3)          # cos of screen angle
        ring_b = BC + 0.07
        w = max(0.7, 0.85 * self.k)
        prof = np.exp(-0.5 * ((rho - ring_b * scale) / w) ** 2)
        halo = np.exp(-0.5 * ((rho - ring_b * scale) / (4.0 * w)) ** 2)
        gr = 1.0 / one_plus_z(7.0, ring_b * theta_x)
        ring_rgb = bb_colour((T0 * np.power(0.95 * gr, T_GAMMA)).ravel()).reshape(H, W, 3)
        self.ring = ((2.4 * prof + 0.25 * halo) * gr ** 4)[..., None] * ring_rgb
        # the smooth sky wash is not lensed, so dim it near the hole; lensed stars stay
        self.wash = (1.0 - 0.75 * np.exp(-(rho / (11.0 * scale)) ** 2))[..., None]
        self.build_time = time.time() - t0

    def _project(self, r, phi, sign, table):
        cpsi = -math.sin(INC) * np.sin(phi)
        psi = np.arccos(np.clip(cpsi, -1, 1))
        ex = np.cos(phi)
        ey = math.cos(INC) * np.sin(phi)
        nrm = np.hypot(ex, ey)
        ex, ey = ex / nrm, ey / nrm
        b = self.lens.lookup(r[:, None], psi[None, :], table).astype(np.float32)
        xs = sign * b * ex[None, :]
        ys = sign * b * ey[None, :]
        return (self.cx + self.s * xs).astype(np.float32), (self.cy - self.s * ys).astype(np.float32)

    def _build_layer(self, r, phi, sign, table):
        X, Y = self._project(r, phi, sign, table)
        xs = (X - self.cx) / self.s
        Xr = np.gradient(X, axis=0)
        Yr = np.gradient(Y, axis=0)
        Xp = 0.5 * (np.roll(X, -1, 1) - np.roll(X, 1, 1))
        Yp = 0.5 * (np.roll(Y, -1, 1) - np.roll(Y, 1, 1))
        area = np.abs(Xr * Yp - Xp * Yr)
        R2 = r[:, None]
        F = page_thorne(r)
        F = F / page_thorne(np.linspace(6.01, 30, 2000)).max()
        F = (F * smoothstep(R_OUT, R_OUT - 10.0, r))[:, None].astype(np.float32)
        g = 1.0 / one_plus_z(R2, xs)
        inten = F * g ** 4
        q = F ** 0.25 * g
        T = T0 * np.power(q, T_GAMMA)
        rgb = bb_colour(T.ravel()).astype(np.float32)
        weights = ((area * inten).ravel()[:, None] * rgb).astype(np.float32)
        sp = Splatter(X, Y, self.W, self.H)
        # outer disk thins out: opacity tapers with the emissivity so the sky shows through
        opac = smoothstep(R_OUT, R_OUT - 9.0, r)[:, None].astype(np.float32)
        cov = np.clip(sp.splat((area * opac).ravel().astype(np.float32)), 0, 1)
        row = np.clip(np.round((r - R_IN) / (R_OUT - R_IN) * self.tex_nr - 0.5), 0,
                      self.tex_nr - 1).astype(np.int32)
        return dict(sp=sp, w=weights, cov=cov, r=r, phi=phi, row=row,
                    omega=(1.0 / r ** 1.5))

    def _texture(self, rng):
        """Turbulent emissivity in the co-rotating frame: streaks long in phi, short in r."""
        nr_c, nphi_c = 320, 2048
        dr = (R_OUT - R_IN) / nr_c
        dphi = 2 * math.pi / nphi_c
        total = np.zeros((nr_c, nphi_c), np.float32)
        for sr, sp, wgt in ((0.07, 0.05, 0.55), (0.2, 0.16, 0.8), (0.6, 0.5, 0.6),
                            (1.6, 1.2, 0.35)):
            n = rng.standard_normal((nr_c, nphi_c)).astype(np.float32)
            n = ndimage.gaussian_filter(n, (sr / dr, sp / dphi), mode=("nearest", "wrap"))
            total += wgt * n / n.std()
        total /= total.std()
        rc = R_IN + (R_OUT - R_IN) * (np.arange(nr_c) + 0.5) / nr_c
        band = 0.35 * np.sin(2 * math.pi * rc / 0.55 + 3 * np.sin(rc * 0.7))[:, None]
        tex = np.exp(0.55 * total + 0.25 * band)
        tex /= tex.mean()
        ri = (np.arange(self.tex_nr) + 0.5) / self.tex_nr * nr_c - 0.5
        pj = (np.arange(self.tex_nphi) + 0.5) / self.tex_nphi * nphi_c - 0.5
        texw = np.concatenate([tex, tex[:, :1]], 1)
        fine = ndimage.map_coordinates(texw, np.meshgrid(ri, np.mod(pj, nphi_c), indexing="ij"),
                                       order=1, mode="nearest")
        return fine.astype(np.float32)

    def texture_at(self, layer, t):
        """Emissivity after coordinate time t: each annulus rotated by Omega(r) t."""
        n = self.tex_nphi
        u = (layer["phi"][None, :] - layer["omega"][:, None] * t) * (n / (2 * math.pi))
        u = np.mod(u, n).astype(np.float32)
        j0 = u.astype(np.int32)
        f = u - j0
        j0 = j0 % n
        rows = self.tex[layer["row"]]
        a = np.take_along_axis(rows, j0, 1)
        b = np.take_along_axis(rows, (j0 + 1) % n, 1)
        return (a * (1 - f) + b * f).ravel()

    def disk_hdr(self, t, gain):
        out = []
        for layer in self.layers:
            tex = self.texture_at(layer, t) * gain
            out.append(layer["sp"].splat(layer["w"] * tex[:, None]))
        return out


def sky_layer(scene, rng, extra=()):
    return S.sky(scene.W, scene.H) * scene.wash + star_layer(scene, rng, extra=extra)


def star_layer(scene, rng, n=5200, extra=()):
    """Background stars on a shell STAR_SHELL behind the hole, each with a primary and a
    secondary image, drawn as small Gaussians stretched by the local lens Jacobian."""
    W, H, s = scene.W, scene.H, scene.s
    lens = scene.lens
    canvas = np.zeros((H, W, 3), np.float32)
    beta = STAR_SHELL * np.sqrt(rng.uniform(0, 1, n)) * 0.999
    th = rng.uniform(0, 2 * math.pi, n)
    mag = rng.pareto(1.6, n) + 1.0
    flux = np.clip(mag, 1, 40) * 0.2
    temp = np.exp(rng.normal(np.log(6500), 0.45, n))
    for b_, t_, f_, T_ in extra:
        beta = np.append(beta, b_)
        th = np.append(th, t_)
        flux = np.append(flux, f_)
        temp = np.append(temp, T_)
    col = bb_colour(temp)

    def world(bt, tt):
        cz = np.sqrt(np.clip(1 - (bt / STAR_SHELL) ** 2, 0, 1))
        return STAR_SHELL * (-cz[:, None] * O[None, :]
                             + (bt / STAR_SHELL)[:, None] * (np.cos(tt)[:, None] * EX
                                                             + np.sin(tt)[:, None] * UP))
    P = world(beta, th)
    d = 0.02
    base = lens.images(P)
    radial = lens.images(world(beta + d, th))
    tang = lens.images(world(beta, th + d / np.maximum(beta, 1e-3)))
    sig0 = 0.55 * max(scene.k, 0.6)
    for img in (0, 1):
        x = base[2 * img]
        y = base[2 * img + 1]
        rho = np.hypot(x, y)
        mu_r = np.hypot(radial[2 * img] - x, radial[2 * img + 1] - y) / d
        mu_t = np.hypot(tang[2 * img] - x, tang[2 * img + 1] - y) / d
        X = scene.cx + s * x
        Y = scene.cy - s * y
        for k in range(beta.size):
            if not (-20 < X[k] < W + 20 and -20 < Y[k] < H + 20):
                continue
            mt = min(mu_t[k], 4.0)
            mr = min(max(mu_r[k], 0.25), 3.0)
            amp = flux[k] * mt * mr
            if amp < 0.03:
                continue
            st = sig0 * max(mt, 1.0) ** 0.9
            sr = sig0 * max(mr, 0.6)
            # tangential direction on screen
            cs, sn = -y[k] / max(rho[k], 1e-6), x[k] / max(rho[k], 1e-6)
            tx, ty = cs, -sn                       # screen y is down
            R = int(3 * max(st, sr)) + 2
            x0, x1 = max(int(X[k]) - R, 0), min(int(X[k]) + R + 1, W)
            y0, y1 = max(int(Y[k]) - R, 0), min(int(Y[k]) + R + 1, H)
            if x0 >= x1 or y0 >= y1:
                continue
            yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
            ox, oy = xx + 0.5 - X[k], yy + 0.5 - Y[k]
            ut = ox * tx + oy * ty
            ur = -ox * ty + oy * tx
            g = np.exp(-0.5 * ((ut / st) ** 2 + (ur / sr) ** 2))
            norm = 1.0 / (2 * math.pi * st * sr)
            canvas[y0:y1, x0:x1] += (g * amp * norm * 255.0)[..., None] * col[k]
    return canvas


def lens_jacobian(lens, P, d=0.05):
    """2x2 Jacobians (screen px offsets, y down, per unlensed screen offset) of both images."""
    base = lens.images(P[None])
    ax = lens.images((P + d * EX)[None])
    ay = lens.images((P + d * UP)[None])
    out = []
    for img in (0, 1):
        x, y = base[2 * img][0], base[2 * img + 1][0]
        J = np.array([[(ax[2 * img][0] - x) / d, (ay[2 * img][0] - x) / d],
                      [(ax[2 * img + 1][0] - y) / d, (ay[2 * img + 1][0] - y) / d]])
        F = np.diag([1.0, -1.0])
        out.append(((x, y), F @ J @ F))
    return out


def clip_jacobian(J, lo, hi):
    U, sv, Vt = np.linalg.svd(J)
    sv = np.clip(sv, lo, hi)
    return U @ np.diag(sv) @ Vt


def lensed_sphere(canvas, x, y, radius, J, colour, alpha=1.0, light=(-0.5, -0.6), emissive=0.0):
    """S.sphere, drawn through a 2x2 lens Jacobian so the image stretches like the lensed body."""
    H, W, _ = canvas.shape
    sv = np.linalg.svd(J, compute_uv=False)
    R = int(math.ceil(radius * sv[0])) + 2
    x0, x1 = max(int(x) - R, 0), min(int(x) + R + 1, W)
    y0, y1 = max(int(y) - R, 0), min(int(y) + R + 1, H)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    ox, oy = xx + 0.5 - x, yy + 0.5 - y
    Ji = np.linalg.inv(J)
    dx = (Ji[0, 0] * ox + Ji[0, 1] * oy) / radius
    dy = (Ji[1, 0] * ox + Ji[1, 1] * oy) / radius
    d2 = dx * dx + dy * dy
    dz = np.sqrt(np.clip(1 - d2, 0, 1))
    lx, ly = light
    lz = math.sqrt(max(0.0, 1 - lx * lx - ly * ly))
    diffuse = np.clip(dx * lx + dy * ly + dz * lz, 0, 1)
    spec = np.clip(dx * lx * 0.5 + dy * ly * 0.5 + dz * (lz * 0.5 + 0.5), 0, 1) ** 30
    col = np.array(colour, np.float32)
    shade = col * (0.25 + emissive + 0.85 * diffuse[..., None]) + 255 * 0.5 * spec[..., None]
    edge = np.clip((1.0 - np.sqrt(d2)) * radius * sv[-1], 0, 1)[..., None] * alpha
    region = canvas[y0:y1, x0:x1]
    region[:] = region * (1 - edge) + shade * edge


def ring_marker(canvas, x, y, radius, colour, k):
    H, W, _ = canvas.shape
    R = int(radius + 4)
    x0, x1 = max(int(x) - R, 0), min(int(x) + R + 1, W)
    y0, y1 = max(int(y) - R, 0), min(int(y) + R + 1, H)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    d = np.hypot(xx + 0.5 - x, yy + 0.5 - y)
    a = np.clip(1.0 - np.abs(d - radius) / max(0.9 * k, 0.7), 0, 1)[..., None] * 0.9
    region = canvas[y0:y1, x0:x1]
    region[:] = region * (1 - a) + np.array(colour, np.float32) * a


def aniso_glow(canvas, x, y, sx, sy, angle, colour, strength=1.0):
    H, W, _ = canvas.shape
    R = int(3 * max(sx, sy)) + 2
    x0, x1 = max(int(x) - R, 0), min(int(x) + R + 1, W)
    y0, y1 = max(int(y) - R, 0), min(int(y) + R + 1, H)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    ox, oy = xx + 0.5 - x, yy + 0.5 - y
    c, s = math.cos(angle), math.sin(angle)
    u = ox * c + oy * s
    v = -ox * s + oy * c
    g = np.exp(-0.5 * ((u / sx) ** 2 + (v / sy) ** 2)) * strength
    canvas[y0:y1, x0:x1] += g[..., None] * np.array(colour, np.float32)


def text_label(draw, x, y, text, size, fill=S.TEXT, anchor="lm"):
    f = S.font(size)
    draw.text((x, y), text, font=f, fill=fill, anchor=anchor, stroke_width=2,
              stroke_fill=(6, 6, 12))


def status_panel(image, lines, size, colours=None):
    """S.status with optional per-line colours."""
    w, h = image.size
    line_h = int(size * 1.45)
    band = line_h * len(lines) + 10
    draw = ImageDraw.Draw(image)
    draw.rectangle([0, h - band, w, h], fill=S.PANEL)
    f = S.font(size)
    for i, text in enumerate(lines):
        c = (colours[i] if colours and colours[i] else (S.TEXT if i == 0 else S.DIM))
        draw.text((8, h - band + 5 + i * line_h), text, font=f, fill=c)
    return band


TOE = 1.3


def tonemap(hdr):
    return 255.0 * srgb(1.0 - np.exp(-EXPOSURE * np.power(np.maximum(hdr, 0), TOE)))


def bloom(hdr, k):
    hot = np.maximum(hdr - 1.0, 0)
    out = np.zeros_like(hdr)
    for sig, w in ((2.0, 0.25), (9.0, 0.12), (30.0, 0.06)):
        for c in range(3):
            out[..., c] += w * ndimage.gaussian_filter(hot[..., c], sig * k, mode="constant")
    return out


def compose(scene, sky_stars, t, gain, extra_hdr=None):
    prim, sec = scene.disk_hdr(t, gain)
    a1 = scene.layers[0]["cov"][..., None]
    a2 = scene.layers[1]["cov"][..., None]
    back = sec + scene.ring
    if extra_hdr is not None:
        back = back + extra_hdr
    hdr = back * (1 - a1) + prim
    hdr = hdr + bloom(hdr, scene.k)
    sky = sky_stars * (1 - scene.shadow[..., None]) * (1 - a1) * (1 - a2)
    return sky + tonemap(hdr)




def orbit_mask(scene, r, dash=(7, 5), front_only=False):
    """Lensed primary image of a circular orbit, as an antialiased dashed polyline mask."""
    ss = 2
    phi = np.linspace(0, 2 * math.pi, 1441)
    P = np.stack([r * np.cos(phi), r * np.sin(phi), np.zeros_like(phi)], -1)
    x1, y1, _, _ = scene.lens.images(P)
    X = (scene.cx + scene.s * x1) * ss
    Y = (scene.cy - scene.s * y1) * ss
    mask = Image.new("L", (scene.W * ss, scene.H * ss), 0)
    d = ImageDraw.Draw(mask)
    seg = np.hypot(np.diff(X), np.diff(Y))
    acc = np.concatenate([[0], np.cumsum(seg)]) / ss
    period = dash[0] + dash[1]
    for k in range(len(X) - 1):
        if (acc[k] % period) < dash[0]:
            d.line([(X[k], Y[k]), (X[k + 1], Y[k + 1])], fill=255, width=ss)
    mask = mask.resize((scene.W, scene.H), Image.BOX)
    return np.asarray(mask, np.float32) / 255.0


def proc_radius_px(p, scene):
    return scene.k * (3.2 + 6.0 * math.sqrt(min(p["rss"], 3400) / 3400.0))


def world_pos(r, phi):
    return np.array([r * math.cos(phi), r * math.sin(phi), 0.0])


def draw_bodies(canvas, scene, bodies, a1):
    """bodies: list of dict(p, r, phi, [plunge]). Draws secondary then primary images."""
    lens = scene.lens
    placed = []
    infos = []
    for b in bodies:
        P = world_pos(b["r"], b["phi"])
        (im1, J1), (im2, J2) = lens_jacobian(lens, P)
        b["X"] = scene.cx + scene.s * im1[0]
        b["Y"] = scene.cy - scene.s * im1[1]
        b["X2"] = scene.cx + scene.s * im2[0]
        b["Y2"] = scene.cy - scene.s * im2[1]
        b["J1"], b["J2"] = J1, J2
        b["depth"] = float(P @ O)
        infos.append(b)
    # secondary images: faint, behind the near side of the disk
    for b in infos:
        if b.get("plunge"):
            continue
        rad = proc_radius_px(b["p"], scene)
        J = clip_jacobian(b["J2"], 0.08, 1.4)
        xi, yi = int(b["X2"]), int(b["Y2"])
        if 0 <= xi < scene.W and 0 <= yi < scene.H:
            vis = 1.0 - float(a1[yi, xi, 0])
            if vis > 0.05:
                lensed_sphere(canvas, b["X2"], b["Y2"], rad, J, S.KIND[b["p"]["kind"]],
                              alpha=0.75 * vis, emissive=0.2)
    for b in sorted(infos, key=lambda q: q["depth"]):
        p = b["p"]
        rad = proc_radius_px(p, scene)
        col = S.KIND[p["kind"]]
        if b.get("plunge"):
            continue
        J = clip_jacobian(b["J1"], 0.8, 1.6)
        S.glow(canvas, b["X"], b["Y"], rad * 1.7, col, 0.28)
        lensed_sphere(canvas, b["X"], b["Y"], rad, J, col)
        if b.get("victim"):
            ring_marker(canvas, b["X"], b["Y"], rad + 4.5 * scene.k, S.ZOMBIE, scene.k)
        placed.append((b["X"] - rad, b["Y"] - rad, b["X"] + rad, b["Y"] + rad))
    for b in infos:
        if not b.get("plunge"):
            continue
        pl = b["plunge"]
        rad = proc_radius_px(b["p"], scene)
        # lens Jacobian plus a radial stretch that grows toward the horizon
        J = clip_jacobian(b["J1"], 0.6, 1.5)
        dx, dy = b["X"] - scene.cx, b["Y"] - scene.cy
        n = math.hypot(dx, dy)
        ux, uy = (dx / n, dy / n) if n > 1e-3 else (1.0, 0.0)
        st = pl["stretch"]
        Sm = np.eye(2) + (st - 1) * np.outer([ux, uy], [ux, uy])
        sq = 1.0 / math.sqrt(st)
        Sm = Sm + (sq - 1) * np.outer([-uy, ux], [-uy, ux])
        Jp = Sm @ J
        bright = pl["bright"]
        col = np.array(pl["colour"], np.float32)
        ang = math.atan2(uy, ux)
        aniso_glow(canvas, b["X"], b["Y"], rad * st * 1.8, rad * sq * 1.5, ang, col,
                   0.9 * bright)
        aniso_glow(canvas, b["X"], b["Y"], rad * (1 + st) * 1.2, rad * sq * 2.5, ang, col,
                   0.3 * bright)
        if bright > 0.02:
            lensed_sphere(canvas, b["X"], b["Y"], rad, Jp, tuple(col * min(1.0, 0.4 + bright)),
                          alpha=min(1.0, bright * 1.6), emissive=0.5)
    return infos, placed


def place_labels(img_arr, items, placed, size, W, H, prev=None, panel_top=None):
    """items: (key, x, y, rad, text). Greedy placement avoiding overlaps and bright areas."""
    f = S.font(size)
    lum = img_arr.mean(axis=2)
    boxes = list(placed)
    out = []
    panel_top = panel_top or H
    for key, x, y, rad, text in items:
        tw = f.getlength(text)
        th = size
        g = rad + 4
        cands = [("r", x + g, y - g * 0.6), ("l", x - g - tw, y - g * 0.6),
                 ("r", x + g, y + g * 0.9), ("l", x - g - tw, y + g * 0.9),
                 ("r", x - tw / 2, y - g - th * 0.8), ("r", x - tw / 2, y + g + th * 0.8)]
        best, best_s = None, 1e18
        for ci, (_, lx, ly) in enumerate(cands):
            box = (lx - 3, ly - th / 2 - 3, lx + tw + 3, ly + th / 2 + 3)
            s = 0.0
            if box[0] < 4 or box[2] > W - 4 or box[1] < 4 or box[3] > panel_top - 4:
                s += 1e9
            for bb in boxes:
                ox = min(box[2], bb[2]) - max(box[0], bb[0])
                oy = min(box[3], bb[3]) - max(box[1], bb[1])
                if ox > 0 and oy > 0:
                    s += 1e6 + ox * oy * 100
            xi0, xi1 = int(max(box[0], 0)), int(min(box[2], W))
            yi0, yi1 = int(max(box[1], 0)), int(min(box[3], H))
            if xi1 > xi0 and yi1 > yi0:
                s += float(lum[yi0:yi1, xi0:xi1].mean()) * 40
            s += ci * 120
            if prev is not None and prev.get(key) == ci:
                s -= 2500
            if s < best_s:
                best, best_s = (ci, lx, ly, box), s
        ci, lx, ly, box = best
        boxes.append(box)
        out.append((key, ci, lx, ly, text))
    return out


def frame(scene, sky_stars, t, gain, bodies, labels, status_lines, status_size, label_size,
          prev_labels=None, status_colours=None, orbit_masks=()):
    canvas = compose(scene, sky_stars, t, gain)
    a1 = scene.layers[0]["cov"][..., None]
    for mask, col, alpha in orbit_masks:
        m = (mask * alpha)[..., None]
        canvas = canvas * (1 - m) + np.array(col, np.float32) * m
    infos, placed = draw_bodies(canvas, scene, bodies, a1)
    img = S.to_image(canvas)
    band = status_panel(img, status_lines, status_size, status_colours)
    by_name = {b["p"]["name"]: b for b in infos}
    items = []
    for name, text in labels:
        b = by_name.get(name)
        if b is None:
            continue
        items.append((name, b["X"], b["Y"], proc_radius_px(b["p"], scene), text))
    arr = np.asarray(img, np.float32)
    placed_l = place_labels(arr, items, placed, label_size, scene.W, scene.H,
                            prev=prev_labels, panel_top=scene.H - band)
    draw = ImageDraw.Draw(img)
    for key, ci, lx, ly, text in placed_l:
        text_label(draw, lx, ly, text, label_size)
    return img, {k: ci for k, ci, *_ in placed_l}


# ----------------------------------------------------------------------------------------------
# Still and animation
# ----------------------------------------------------------------------------------------------
STILL_PHASES = {"worker-134": -134, "browser-512": 72, "compiler-90": -14,
                "language-server-71": 196, "postgres-41": -57, "systemd-1": -104,
                "redis-207": 150, "database-141": 112}

LEGEND = ("orbit radius = oom_score rank (ISCO 6M = next victim) | disk brightness = memory "
          "pressure | colour = Doppler + gravitational redshift | far orbit = oom_score_adj -1000")

EXTRA_STARS = [(1.6, 0.9, 9.0, 9000.0), (3.2, 2.6, 6.0, 4200.0), (2.4, 4.4, 5.0, 12000.0),
               (5.5, 5.6, 4.0, 5200.0)]


UNKILLABLE_DR = {"systemd-1": 0.0, "sshd-702": 0.8, "udevd-333": -0.8}


def bodies_static(procs, n_kill, phases=None):
    out = []
    for p in procs:
        if p["adj"] <= -1000:
            r = UNKILLABLE_R + UNKILLABLE_DR[p["name"]]
        elif p["rank"] <= N_ORBIT:
            r = rank_radius(p["rank"])
        else:
            continue
        phi = p["phi"]
        if phases and p["name"] in phases:
            phi = math.radians(phases[p["name"]])
        out.append(dict(p=p, r=r, phi=phi))
    return out


def label_text(p):
    if p["adj"] <= -1000:
        return "%s unkillable" % p["name"]
    return "%s %d" % (p["name"], p["score"])


def make_still():
    W, H = 1600, 900
    lens = LensTable()
    scale = 23.5
    scene = Scene(W, H, scale, (800.0, 432.0), lens)
    print("scene built in %.1fs" % scene.build_time, flush=True)
    rng = np.random.default_rng(42)
    skyst = sky_layer(scene, rng, extra=EXTRA_STARS)
    procs, n_kill = demo_processes()
    for p in procs:
        if p["adj"] <= -1000:
            p["r_fixed"] = UNKILLABLE_R
    bodies = bodies_static(procs, n_kill, STILL_PHASES)
    for b in bodies:
        b["victim"] = b["p"]["name"] == "worker-134"
    names = ["worker-134", "browser-512", "compiler-90", "postgres-41", "language-server-71",
             "systemd-1"]
    by = {p["name"]: p for p in procs}
    labels = [(n, label_text(by[n])) for n in names]
    pressure = 0.62
    status = [
        "ISOTOP / HORIZON / DEMO   192 processes | top %d by oom_score in orbit + 3 unkillable | "
        "RAM 14.9 / 16.0 GiB | pressure mem some avg10 %d%% | oom_kill 2 | i = 80 deg"
        % (N_ORBIT, pressure * 100),
        LEGEND,
        "next victim: worker-134  oom_score 912  rank 1  r = 6.00 M  period 92 M | lensed: disk, "
        "orbits, stars (primary + secondary images) | photon ring drawn, not traced",
    ]
    isco = orbit_mask(scene, 6.0)
    outer = orbit_mask(scene, UNKILLABLE_R, dash=(3, 6))
    t = 37.0
    img, _ = frame(scene, skyst, t, 0.45 + 0.9 * pressure, bodies, labels, status, 13, 15,
                   orbit_masks=[(isco, (230, 225, 255), 0.45), (outer, (180, 190, 220), 0.25)])
    # ISCO tag
    path = S.save_still(img, "horizon")
    print("saved", path)


def ease(x):
    x = min(max(x, 0.0), 1.0)
    return x * x * (3 - 2 * x)


LEGEND_SHORT = ("orbit radius = oom_score rank (ISCO = next victim) | disk brightness = memory "
                "pressure | colour = Doppler + gravitational redshift")


def make_anim(n_frames=120, fps=12, only=None):
    W, H = 960, 540
    lens = LensTable()
    k = 0.6
    scene = Scene(W, H, 23.5 * k, (480.0, 259.0), lens, max_samples=3.0e6)
    print("scene built in %.1fs" % scene.build_time, flush=True)
    rng = np.random.default_rng(42)
    skyst = sky_layer(scene, rng, extra=EXTRA_STARS)
    procs, n_kill = demo_processes()
    by = {p["name"]: p for p in procs}
    dt = M_PER_SECOND / fps
    t_pre = 20.0
    kill_s = 3.5
    t_kill = t_pre + kill_s * M_PER_SECOND
    ts, rs, ps = plunge_track()
    # worker-134 leaves the ISCO from the far right so it swings over the top and ends in front
    phi_kill = math.radians(-90.0) - ps[-1]
    phases = dict(STILL_PHASES)
    phases["worker-134"] = math.degrees(phi_kill - t_kill / 6.0 ** 1.5)
    bodies = bodies_static(procs, n_kill, phases)
    for b in bodies:
        b["r0"] = b["r"]
        b["phi"] += t_pre / b["r"] ** 1.5
    isco = orbit_mask(scene, 6.0, dash=(5, 4))
    outer = orbit_mask(scene, UNKILLABLE_R, dash=(2, 5))
    names = ["worker-134", "browser-512", "compiler-90", "postgres-41", "systemd-1"]
    frames, prev = [], None
    pk = None
    for f in range(n_frames):
        sec = f / fps
        t = t_pre + sec * M_PER_SECOND
        after = t >= t_kill
        # memory pressure climbs toward the kill, then relaxes
        if not after:
            pressure = 0.62 + 0.30 * ease(sec / kill_s) ** 2
        else:
            pressure = 0.92 - 0.50 * ease((sec - kill_s) / 1.6)
        gain = 0.45 + 0.9 * pressure
        promote = ease((sec - kill_s - 2.4) / 2.8) if after else 0.0
        draw = []
        for b in bodies:
            p = b["p"]
            b.pop("plunge", None)
            b["victim"] = False
            if p["name"] == "worker-134":
                if not after:
                    b["r"] = 6.0
                    b["phi"] += dt / 6.0 ** 1.5
                    b["victim"] = True
                else:
                    tau = t - t_kill
                    r = float(np.interp(tau, ts, rs))
                    b["r"] = max(r, 2.0006)
                    b["phi"] = phi_kill + float(np.interp(tau, ts, ps))
                    g2 = (1 - 2 / b["r"]) / (2.0 / 3.0)       # e-folds every 4M near 2M
                    bright = math.sqrt(max(g2, 0.0))   # display gamma; g2 itself e-folds in 4M
                    red = 1.0 - math.sqrt(max(g2, 0.0))
                    col = (1 - red) * np.array(S.KIND["container"], float) \
                        + red * np.array((235, 60, 30), float)
                    stretch = 1.0 + 2.6 * min(1.0, (6.0 - b["r"]) / 4.0) ** 1.5
                    b["plunge"] = dict(bright=bright, colour=col, stretch=stretch)
                    pk = (b["r"], bright)
                    if bright < 0.06:
                        continue
                draw.append(b)
                continue
            if p["adj"] > -1000 and after:
                target = rank_radius(p["rank"] - 1)
                b["r"] = b["r0"] + (target - b["r0"]) * promote
            b["phi"] += dt / b["r"] ** 1.5
            if p["name"] == "browser-512" and after and promote > 0.85:
                b["victim"] = True
            draw.append(b)
        labels = []
        for n in names:
            p = by[n]
            if n == "worker-134" and after:
                if pk and pk[1] > 0.3:
                    labels.append((n, "worker-134 killed"))
                continue
            labels.append((n, label_text(p)))
        nproc = 192 if not after else 191
        status = ["ISOTOP / HORIZON / DEMO   %d processes | RAM %.1f / 16.0 GiB | pressure mem "
                  "some avg10 %d%% | oom_kill %d" % (nproc, 15.7 if not after else 12.4,
                                                     round(pressure * 100), 2 if not after else 3),
                  LEGEND_SHORT]
        colours = [None, None, None]
        if not after:
            status.append("next victim: worker-134  oom_score 912  rank 1  r = 6.00 M  "
                          "period 92 M")
        else:
            line = "OOM: killed worker-134 (oom_score 912)"
            if pk and pk[1] > 0.06:
                if pk[0] > 2.12:
                    line += "   plunging inside the ISCO, r = %.2f M" % pk[0]
                else:
                    line += "   image frozen at r = %.3f M, never crosses 2M" % pk[0]
            if promote > 0.85:
                line += "   next victim: browser-512 (887) eased in to the ISCO"
            status.append(line)
            colours[2] = S.ZOMBIE
        if only is not None and f not in only:
            continue
        img, prev = frame(scene, skyst, t, gain, draw, labels, status, 11, 11, prev_labels=prev,
                          status_colours=colours,
                          orbit_masks=[(isco, (230, 225, 255), 0.4), (outer, (180, 190, 220), 0.22)])
        frames.append(img)
        if f % 12 == 0:
            print("frame %d" % f, flush=True)
    return frames


def contact_sheet(frames, path, idx):
    w, h = frames[0].size
    sheet = Image.new("RGB", (w * 2, h * 2))
    for n, i in enumerate(idx):
        sheet.paste(frames[i], ((n % 2) * w, (n // 2) * h))
    sheet.save(path)


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "still"
    t0 = time.time()
    if mode == "still":
        make_still()
    elif mode == "anim":
        frames = make_anim()
        path, size = S.save_frames(frames, "horizon", fps=12)
        # S.save_frames' PIL GIF comes out ~6 MB for a rotating disk; re-encode the saved frames
        # with an ffmpeg diff palette and ordered dither to land under 3.5 MB at 720 wide.
        import subprocess
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-framerate", "12", "-i",
                        os.path.join(S.OUT, "horizon-frames", "%04d.png"), "-vf",
                        "scale=720:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=112:"
                        "stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4:"
                        "diff_mode=rectangle", path], check=True)
        print("gif", path, os.path.getsize(path))
        contact_sheet(frames, os.path.join(S.OUT, "horizon-sheet.png"), [10, 50, 75, 112])
    elif mode == "probe":
        only = [int(x) for x in sys.argv[2].split(",")]
        frames = make_anim(only=only)
        contact_sheet(frames, os.path.join(S.OUT, "horizon-sheet.png"), list(range(4)))
    elif mode == "test":
        lens = LensTable()
        psi = np.linspace(0.1, 2.0, 20)
        b = lens.lookup(np.full_like(psi, 10.0), psi, lens.B1)
        err = np.abs(b / beloborodov_b(10.0, psi) - 1).max()
        r = np.linspace(6, 30, 50)[:, None]
        sec = lens.lookup(r + 0 * psi, psi + 0 * r, lens.B2)
        print("max rel err vs Beloborodov at r=10M, psi<=2: %.4f; min secondary b %.4f (b_c %.4f)"
              % (err, sec.min(), BC))
    print("done in %.1fs" % (time.time() - t0))
