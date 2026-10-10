"""Mockup of isotop's die view: the CPU die from above in FLIR ironbow false colour.

A schematic Intel hybrid floorplan (6 P-cores, 2 E-core clusters of 4, L3 slices on a ring,
iGPU, system agent) carries a real 2D heat equation,
    dT/dt = kappa lap(T) + P/C - (T - T_amb)/tau,
with each block's power proxy P ~ busy x (f/f_max)^3. A demo compile lights the cores in
sequence, the preferred core P2 crosses 100 C and throttles, then everything cools.

usage: python3 -I die.py [still|anim|test]
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import isostyle as S  # noqa: E402

import cv2  # noqa: E402
import numpy as np  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402
from scipy import ndimage  # noqa: E402

NX, NY = 320, 160                 # die grid (cells)
KAPPA = 0.2                       # cells^2 per step (explicit scheme stable for < 0.25)
TAU = 2600.0                      # sink time constant, steps
T_AMB = 34.0
STEPS_PER_FRAME = 230             # 12 fps animation: ~1 s thermal time constant
T_THROTTLE = 100.0
T_MIN, T_MAX = 30.0, 105.0

# Ironbow: black, deep blue, purple, magenta, red, orange, yellow, white.
IRON = [(0.00, (0, 0, 0)), (0.10, (10, 4, 60)), (0.24, (58, 4, 128)), (0.38, (128, 8, 148)),
        (0.52, (196, 30, 112)), (0.64, (232, 76, 40)), (0.76, (250, 140, 8)),
        (0.88, (255, 206, 40)), (0.96, (255, 246, 160)), (1.00, (255, 255, 250))]
_ix = np.array([s for s, _ in IRON])
_ic = np.array([c for _, c in IRON], np.float32)


_LUT = np.stack([np.interp(np.linspace(0, 1, 2048), _ix, _ic[:, i]) for i in range(3)], -1).astype(np.float32)


def ironbow(t):
    idx = (np.clip(t, 0, 1) * 2047).astype(np.int32)
    return _LUT[idx]


def tnorm(temp):
    return (temp - T_MIN) / (T_MAX - T_MIN)


# ---------------------------------------------------------------- floorplan (unit die coords)
class Block:
    def __init__(self, name, kind, rect, cpus="", fmax=1.0, pmax=1.0, label=None, hot=(0.5, 0.45)):
        self.name, self.kind, self.rect = name, kind, rect
        self.cpus, self.fmax, self.pmax = cpus, fmax, pmax
        self.label = label if label is not None else name
        self.hot = hot
        self.busy, self.freq = 0.0, 0.0
        self.throttles, self.throttle_until, self.flash = 0, -1.0, 0.0
        self.temp = T_AMB

    def power(self):
        return self.pmax * self.busy * (self.freq / self.fmax) ** 3 if self.fmax else 0.0


def floorplan():
    blocks = []
    x0, x1 = 0.165, 0.715            # compute region
    pw = (x1 - x0) / 6
    for i in range(6):
        r = (x0 + i * pw + 0.004, 0.05, x0 + (i + 1) * pw - 0.004, 0.40)
        blocks.append(Block(f"P{i}", "P", r, cpus=f"cpu{2 * i},{2 * i + 1}", fmax=5.4 if i == 2 else 4.9,
                            pmax=11.6 if i == 2 else 10.5, hot=(0.42, 0.40)))
    # L3 slices on the ring, one per ring stop.
    sw = (x1 - x0) / 8
    for i in range(8):
        blocks.append(Block(f"L3.{i}", "L3", (x0 + i * sw + 0.003, 0.43, x0 + (i + 1) * sw - 0.003, 0.57),
                            label=""))
    ew = (x1 - x0) / 2
    for c in range(2):
        cx0 = x0 + c * ew + 0.004
        cx1 = x0 + (c + 1) * ew - 0.004
        blocks.append(Block(f"E-cluster {c}", "Ecl", (cx0, 0.60, cx1, 0.95)))
        cw = (cx1 - cx0) / 4
        for k in range(4):
            n = 12 + c * 4 + k
            blocks.append(Block(f"E{c * 4 + k}", "E", (cx0 + k * cw + 0.006, 0.63, cx0 + (k + 1) * cw - 0.006, 0.82),
                                cpus=f"cpu{n}", fmax=3.8, pmax=3.6, label="", hot=(0.5, 0.5)))
        blocks.append(Block(f"L2.{c}", "L2", (cx0 + 0.006, 0.845, cx1 - 0.006, 0.93), label=""))
    blocks.append(Block("iGPU", "GPU", (0.735, 0.05, 0.985, 0.95), fmax=1.4, pmax=14.0, hot=(0.5, 0.5)))
    blocks.append(Block("SA", "SA", (0.015, 0.05, 0.145, 0.95), fmax=1.0, pmax=3.0, hot=(0.5, 0.6)))
    return blocks


class Die:
    def __init__(self):
        self.blocks = floorplan()
        self.by = {b.name: b for b in self.blocks}
        self.T = np.full((NY, NX), T_AMB + 6.0, np.float32)
        yy, xx = np.mgrid[0:NY, 0:NX].astype(np.float32)
        yy = (yy + 0.5) / NY
        xx = (xx + 0.5) / NX
        rng = np.random.default_rng(3)
        # Per-block normalised source shapes (sum 1), with a hotspot where the execution units are.
        self.shape = {}
        for b in self.blocks:
            x0, y0, x1, y1 = b.rect
            m = ((xx >= x0) & (xx <= x1) & (yy >= y0) & (yy <= y1)).astype(np.float32)
            if b.kind in ("P", "E", "GPU", "SA"):
                hx, hy = x0 + (x1 - x0) * b.hot[0], y0 + (y1 - y0) * b.hot[1]
                sx, sy = (x1 - x0) * 0.28, (y1 - y0) * 0.22
                g = np.exp(-((xx - hx) / sx) ** 2 - ((yy - hy) / sy) ** 2)
                m = m * (0.45 + 1.6 * g)
                if b.kind == "GPU":   # execution-unit array texture
                    m *= 0.8 + 0.4 * ((np.sin(xx * NX * 0.55) > 0) ^ (np.sin(yy * NY * 0.45) > 0))
                m *= 0.92 + 0.16 * rng.random(m.shape)
            if m.sum() > 0:
                self.shape[b.name] = m / m.sum()
        # Gain from watts to K per step: one P-core alone at full power settles near 90 C.
        self.gain = 1.0
        self.gain = 1.0 / self.calibrate()
        self.time = 0.0
        self.rapl = 0.0
        self.hist = []

    def calibrate(self):
        q = self.shape["P2"] * 10.5
        u = np.zeros((NY, NX), np.float32)
        for _ in range(9000):
            u = u + KAPPA * lap(u) + q - u / TAU
        return float(u.max()) / 40.0

    def source(self):
        q = np.zeros((NY, NX), np.float32)
        for b in self.blocks:
            p = b.power()
            if p > 0 and b.name in self.shape:
                q += self.shape[b.name] * p
        return q * self.gain

    def step(self, n):
        q = self.source()
        T = self.T
        for _ in range(n):
            T = T + KAPPA * lap(T) + q - (T - T_AMB) / TAU
        self.T = T
        for b in self.blocks:
            if b.kind in ("P", "E", "GPU", "SA"):
                x0, y0, x1, y1 = b.rect
                hx, hy = x0 + (x1 - x0) * b.hot[0], y0 + (y1 - y0) * b.hot[1]
                b.temp = float(T[int(hy * NY), int(hx * NX)])
        self.rapl = sum(b.power() for b in self.blocks) + 2.2  # + ring, IO and leakage
        self.hist.append((self.package(), self.rapl))
        del self.hist[:-120]

    def cell_temp(self, name):
        return self.by[name].temp

    def package(self):
        return max(b.temp for b in self.blocks if b.kind in ("P", "E"))


def lap(T):
    p = np.pad(T, 1, mode="edge")
    return p[:-2, 1:-1] + p[2:, 1:-1] + p[1:-1, :-2] + p[1:-1, 2:] - 4 * T


# ---------------------------------------------------------------- demo load
ORDER = ["P2", "P3", "P1", "P4", "P0", "P5", "E0", "E1", "E2", "E3", "E4", "E5", "E6", "E7"]
COMPILE_START, COMPILE_END = 0.7, 6.3


def smooth(x):
    x = min(max(x, 0.0), 1.0)
    return x * x * (3 - 2 * x)


def set_load(die, t, rng):
    for b in die.blocks:
        if b.kind == "P":
            b.busy, b.freq = 0.03 + 0.02 * rng.random(), 1.2
        elif b.kind == "E":
            b.busy, b.freq = 0.06 + 0.06 * rng.random(), 1.6
    die.by["P1"].busy, die.by["P1"].freq = 0.22 + 0.05 * rng.random(), 3.4      # browser-512
    die.by["E1"].busy, die.by["E1"].freq = 0.18, 2.4                              # postgres-41
    die.by["iGPU"].busy, die.by["iGPU"].freq = 0.40 + 0.05 * math.sin(t * 3), 1.1  # compositor
    die.by["SA"].busy, die.by["SA"].freq = 1.0, 1.0
    for k, name in enumerate(ORDER):
        b = die.by[name]
        on = smooth((t - (COMPILE_START + 0.16 * k)) / 0.25) * (1 - smooth((t - (COMPILE_END + 0.07 * k)) / 0.3))
        if on <= 0:
            continue
        fmax_run = b.fmax if b.kind == "P" else 3.6
        if b.kind == "P" and name != "P2":
            fmax_run = 4.7
        busy = 0.93 + 0.07 * rng.random()
        b.busy = b.busy * (1 - on) + busy * on
        b.freq = b.freq * (1 - on) + fmax_run * on
        if b.throttles:
            b.freq = min(b.freq, 4.2 if t < b.throttle_until else 4.7)
    for b in die.blocks:
        if b.kind == "P":
            if b.temp >= T_THROTTLE and t >= b.throttle_until:
                b.throttles += 1
                b.throttle_until = t + 1.4
            b.flash = 1.0 if t < b.throttle_until else 0.0


# processes: name, kind, cpu share, block, (u, v) inside the block, labelled
PROCS = [("compiler-90", "session", 1.00, "P2", (0.46, 0.38), True),
         ("browser-512", "session", 0.24, "P1", (0.50, 0.42), True),
         ("postgres-41", "system", 0.18, "E1", (0.50, 0.42), True),
         ("pipewire-95", "session", 0.04, "E5", (0.5, 0.6), False),
         ("systemd-1", "system", 0.01, "E0", (0.4, 0.3), False),
         ("journald-310", "system", 0.02, "E6", (0.6, 0.6), False),
         ("redis-207", "container", 0.03, "E3", (0.5, 0.7), False),
         ("nginx-128", "system", 0.02, "E7", (0.4, 0.4), False),
         ("language-server-71", "session", 0.06, "P4", (0.3, 0.75), False),
         ("worker-134", "container", 0.05, "P5", (0.7, 0.7), False),
         ("shell-170", "session", 0.01, "E2", (0.6, 0.3), False)]


def live_procs(die, t, rng):
    """Process points for this instant: the fixed demo processes plus the compile jobs
    (children of compiler-90, unlabelled) on whichever core each last ran on."""
    out = [p for p in PROCS if p[0] != "compiler-90" or COMPILE_START + 0.1 < t < COMPILE_END + 0.2]
    for k, name in enumerate(ORDER[1:], 1):
        b = die.by[name]
        if b.busy > 0.5:
            out.append((f"cc1plus-{900 + k}", "session", b.busy * 0.95, name,
                        (0.55 + 0.1 * math.sin(k * 2.1), 0.45 + 0.12 * math.cos(k * 1.3)), False))
    return out


# ---------------------------------------------------------------- rendering
LEGEND = ("ironbow = temperature, model between sensors | heat source = busy x (f/fmax)^3 | "
          "points = processes on last core, size = CPU | flashing outline = throttle")
KEYS = "Tab next view | g tour | scroll pan | click inspect | / search | c links | Space pause | ? help | q quit"


class Camera:
    """Perspective tilt of the package plane onto the screen."""

    def __init__(self, w, h, band, panel):
        self.w, self.h = w, h
        scene_w = w - panel
        scene_h = h - band
        cx = scene_w * 0.5 + w * 0.005
        top, bot = scene_h * 0.15, scene_h * 0.90
        half_top, half_bot = scene_w * 0.405, scene_w * 0.455
        self.quad = np.float32([[cx - half_top, top], [cx + half_top, top],
                                [cx + half_bot, bot], [cx - half_bot, bot]])
        self.thick = scene_h * 0.022

    def matrix(self, tw, th, ss=1):
        src = np.float32([[0, 0], [tw, 0], [tw, th], [0, th]])
        return cv2.getPerspectiveTransform(src, self.quad * ss)


# Package texture: die inset on the substrate (package units, die occupies [DX0, DX1] x [DY0, DY1]).
PKG_W, PKG_H = 1.0, 0.56
DX0, DX1, DY0, DY1 = 0.05, 0.95, 0.055, 0.505     # die rectangle within the package (fractions)


def pkg_uv(u, v):
    """Die-unit coords to package fractional coords."""
    return DX0 + u * (DX1 - DX0), (DY0 + v * (DY1 - DY0) / PKG_H)


def die_to_pkg(u, v):
    return DX0 + u * (DX1 - DX0), DY0 / PKG_H + v * (DY1 - DY0) / PKG_H


def texture(die, tw, th, t):
    """Top-down ironbow picture of package + die, with outlines; float RGB (th, tw, 3)."""
    # Substrate temperature: a soft halo of the die field, pulled toward ambient.
    dw = int(round((DX1 - DX0) * tw))
    dh = int(round((DY1 - DY0) / PKG_H * th))
    ox, oy = int(round(DX0 * tw)), int(round(DY0 / PKG_H * th))
    Tdie = cv2.resize(die.T, (dw, dh), interpolation=cv2.INTER_CUBIC)
    sub = np.full((th, tw), T_AMB + 2.0, np.float32)
    k = 8
    small = np.full((th // k, tw // k), T_AMB, np.float32)
    small[oy // k:(oy + dh) // k, ox // k:(ox + dw) // k] = cv2.resize(
        die.T, ((ox + dw) // k - ox // k, (oy + dh) // k - oy // k), interpolation=cv2.INTER_AREA)
    halo = cv2.GaussianBlur(small, (0, 0), tw / 40 / k)
    halo = cv2.resize(halo, (tw, th), interpolation=cv2.INTER_LINEAR)
    yy, xx = np.mgrid[0:th, 0:tw].astype(np.float32)
    sub = T_AMB - 1.0 + (halo - T_AMB) * 0.30
    # Substrate parts read slightly cooler (emissivity): capacitor rows and the edge of the package.
    caps = np.zeros((th, tw), np.float32)
    rng = np.random.default_rng(11)
    for side in range(2):
        y = (oy + dh + th * 0.035) if side == 0 else (oy - th * 0.06)
        for k in range(22):
            x = ox + dw * (0.04 + 0.92 * k / 21)
            cv2.rectangle(caps, (int(x - tw * 0.006), int(y)), (int(x + tw * 0.006), int(y + th * 0.025)), 1.0, -1)
    sub -= caps * 3.5
    edge = np.minimum(np.minimum(xx, tw - 1 - xx), np.minimum(yy, th - 1 - yy)) / (tw * 0.03)
    sub -= (1 - np.clip(edge, 0, 1)) * 2.5
    temp = sub.copy()
    temp[oy:oy + dh, ox:ox + dw] = Tdie
    col = ironbow(tnorm(temp)).astype(np.float32)
    # Thin dark seam around the die (the die edge reads cooler on a FLIR shot).
    cv2.rectangle(col, (ox - 2, oy - 2), (ox + dw + 1, oy + dh + 1), (8, 4, 30), max(2, tw // 600), cv2.LINE_AA)
    # Block outlines: thin, light, partly transparent.
    over = np.zeros_like(col)
    a = np.zeros((th, tw), np.float32)
    lw = max(1, tw // 900)
    for b in die.blocks:
        x0, y0, x1, y1 = b.rect
        p0 = (int(ox + x0 * dw), int(oy + y0 * dh))
        p1 = (int(ox + x1 * dw), int(oy + y1 * dh))
        strength = {"P": 0.55, "E": 0.32, "Ecl": 0.55, "L3": 0.30, "L2": 0.30, "GPU": 0.55, "SA": 0.55}[b.kind]
        colour = (215, 222, 240)
        width = lw
        if b.flash > 0:
            blink = 0.5 + 0.5 * math.cos(t * 2 * math.pi * 3.0)
            colour = (110, 240, 255)
            strength = 0.45 + 0.55 * blink
            width = lw * 3
        cv2.rectangle(over, p0, p1, colour, width, cv2.LINE_AA)
        cv2.rectangle(a, p0, p1, strength, width, cv2.LINE_AA)
    # The ring: a line through the L3 slices.
    ry = int(oy + 0.50 * dh)
    cv2.line(over, (int(ox + 0.155 * dw), ry), (int(ox + 0.725 * dw), ry), (215, 222, 240), lw, cv2.LINE_AA)
    cv2.line(a, (int(ox + 0.155 * dw), ry), (int(ox + 0.725 * dw), ry), 0.25, lw, cv2.LINE_AA)
    # GPU execution-unit grid, faint.
    g = die.by["iGPU"].rect
    for k in range(1, 8):
        x = int(ox + (g[0] + (g[2] - g[0]) * k / 8) * dw)
        cv2.line(a, (x, int(oy + g[1] * dh) + 4), (x, int(oy + g[3] * dh) - 4), 0.10, lw, cv2.LINE_AA)
        cv2.line(over, (x, int(oy + g[1] * dh) + 4), (x, int(oy + g[3] * dh) - 4), (215, 222, 240), lw, cv2.LINE_AA)
    for k in range(1, 4):
        y = int(oy + (g[1] + (g[3] - g[1]) * k / 4) * dh)
        cv2.line(a, (int(ox + g[0] * dw) + 4, y), (int(ox + g[2] * dw) - 4, y), 0.10, lw, cv2.LINE_AA)
        cv2.line(over, (int(ox + g[0] * dw) + 4, y), (int(ox + g[2] * dw) - 4, y), (215, 222, 240), lw, cv2.LINE_AA)
    # Sensor marks: small crosses at each digital thermal sensor.
    for b in die.blocks:
        if b.kind in ("P", "E"):
            x0, y0, x1, y1 = b.rect
            sx = int(ox + (x0 + (x1 - x0) * b.hot[0]) * dw)
            sy = int(oy + (y0 + (y1 - y0) * b.hot[1]) * dh)
            r = max(3, tw // 260) if b.kind == "P" else max(2, tw // 380)
            for dx, dy in ((1, 0), (0, 1)):
                cv2.line(over, (sx - r * dx, sy - r * dy), (sx + r * dx, sy + r * dy), (230, 240, 255), lw, cv2.LINE_AA)
                cv2.line(a, (sx - r * dx, sy - r * dy), (sx + r * dx, sy + r * dy), 0.5, lw, cv2.LINE_AA)
    col = col * (1 - a[..., None]) + over * a[..., None]
    return col, (ox, oy, dw, dh)


def render(die, w, h, t, procs, lines, ss=2, hover=None):
    size = max(11, w // 115)
    band = int(size * 1.45) * len(lines) + 10
    panel = int(w * 0.19)
    cam = Camera(w, h, band, panel)
    canvas = S.sky(w, h)
    W, H = w * ss, h * ss
    big = cv2.resize(canvas, (W, H), interpolation=cv2.INTER_LINEAR)

    tw = int(0.86 * w * ss)
    th = int(tw * PKG_H)
    tex, (ox, oy, dw, dh) = texture(die, tw, th, t)
    M = cam.matrix(tw, th, ss)

    # Drop shadow and package side faces.
    q = cam.quad * ss
    shadow = np.zeros((H, W), np.float32)
    sq = q.copy()
    sq[:, 1] += cam.thick * ss * 1.8
    sq[:, 0] += w * ss * 0.006
    cv2.fillConvexPoly(shadow, np.int32(sq * 16), 1.0, cv2.LINE_AA, 4)
    shadow = cv2.GaussianBlur(shadow, (0, 0), w * ss / 70)
    big *= (1 - 0.65 * shadow)[..., None]
    th_px = cam.thick * ss
    front = np.float32([q[3], q[2], q[2] + [0, th_px], q[3] + [0, th_px]])
    cv2.fillConvexPoly(big, np.int32(front * 16), (16, 22, 18), cv2.LINE_AA, 4)
    left = np.float32([q[0], q[3], q[3] + [0, th_px], q[0] + [0, th_px]])
    cv2.fillConvexPoly(big, np.int32(left * 16), (10, 12, 12), cv2.LINE_AA, 4)
    right = np.float32([q[1], q[2], q[2] + [0, th_px], q[1] + [0, th_px]])
    cv2.fillConvexPoly(big, np.int32(right * 16), (22, 26, 24), cv2.LINE_AA, 4)
    # A thin edge highlight on the front face's top lip.
    cv2.line(big, tuple(np.int32(q[3] * 16)), tuple(np.int32(q[2] * 16)), (90, 70, 120), ss, cv2.LINE_AA, 4)

    warped = cv2.warpPerspective(tex, M, (W, H), flags=cv2.INTER_AREA if False else cv2.INTER_LINEAR,
                                 borderMode=cv2.BORDER_CONSTANT, borderValue=(0, 0, 0))
    mask = cv2.warpPerspective(np.ones((th, tw), np.float32), M, (W, H), flags=cv2.INTER_LINEAR)
    big = big * (1 - mask[..., None]) + warped * mask[..., None]

    # Bloom: hot pixels glow into the air above the die.
    lum = np.clip((warped.max(axis=2) - 170) / 85.0, 0, 1) * mask
    lum_small = cv2.resize(lum, (W // 4, H // 4), interpolation=cv2.INTER_AREA)
    b1 = cv2.GaussianBlur(lum_small, (0, 0), w / 260)
    b2 = cv2.GaussianBlur(lum_small, (0, 0), w / 70)
    b1 = cv2.resize(b1, (W, H), interpolation=cv2.INTER_LINEAR)
    b2 = cv2.resize(b2, (W, H), interpolation=cv2.INTER_LINEAR)
    big += b1[..., None] * np.float32([120, 70, 20]) * 0.75
    big += b2[..., None] * np.float32([150, 60, 40]) * 0.6

    # Process points, projected through the same camera.
    def to_screen(block, u, v):
        x0, y0, x1, y1 = die.by[block].rect
        du, dv = x0 + (x1 - x0) * u, y0 + (y1 - y0) * v
        px, py = ox + du * dw, oy + dv * dh
        p = cv2.perspectiveTransform(np.float32([[[px, py]]]), M)[0, 0]
        return float(p[0]), float(p[1])

    pts = []
    for name, kind, cpu, block, (u, v), labelled in procs:
        x, y = to_screen(block, u, v)
        r = (1.6 + 4.2 * math.sqrt(max(cpu, 0.0))) * w / 1600 * ss
        cv2.circle(big, (int(x * 16), int(y * 16)), int((r + 1.6 * ss) * 16), (12, 6, 30), -1, cv2.LINE_AA, 4)
        cv2.circle(big, (int(x * 16), int(y * 16)), int((r + 0.8 * ss) * 16), S.KIND[kind], -1, cv2.LINE_AA, 4)
        cv2.circle(big, (int(x * 16), int(y * 16)), int(r * 0.7 * 16), (255, 255, 255), -1, cv2.LINE_AA, 4)
        pts.append((name, cpu, x / ss, y / ss, labelled, block))

    img = S.to_image(cv2.resize(big, (w, h), interpolation=cv2.INTER_AREA))
    draw = ImageDraw.Draw(img)
    fs = max(11, int(w / 108))
    fsl = max(10, int(w / 125))

    def scr(u, v):
        p = cv2.perspectiveTransform(np.float32([[[ox + u * dw, oy + v * dh]]]), M)[0, 0] / ss
        return float(p[0]), float(p[1])

    # Block labels: tucked in the top-left corner of each block.
    for b in die.blocks:
        if not b.label:
            continue
        x0, y0, x1, y1 = b.rect
        if b.kind in ("P",):
            x, y = scr(x0 + 0.010, y0 + 0.022)
            lab(draw, x, y, b.label, fs, (240, 244, 255) if not b.flash else (150, 245, 255), anchor="la", bold=True)
        else:
            x, y = scr(x0 + 0.010, y0 + 0.022)
            lab(draw, x, y, b.label, fs, (230, 234, 245), anchor="la", bold=True)
    x, y = scr(0.155 + 0.004, 0.462)
    lab(draw, x, y, "L3", fsl, (220, 224, 240), anchor="la")
    x, y = scr(0.735 + 0.004, 0.395)
    lab(draw, x, y, "ring", fsl, (220, 224, 240), anchor="ra")
    for c in range(2):
        x, y = scr(0.165 + c * 0.275 + 0.014, 0.858)
        lab(draw, x, y, "L2", fsl, (220, 224, 240), anchor="la")
    if any(b.flash for b in die.blocks):
        b = next(b for b in die.blocks if b.flash)
        x, y = scr((b.rect[0] + b.rect[2]) / 2, b.rect[1] - 0.01)
        n = b.throttles
        lab(draw, x, y - fs * 0.3, f"THROTTLE  {b.name}  x{n}", fsl, (150, 245, 255), anchor="md", bold=True)

    # Package caption.
    x, y = scr(1.0, 1.0)
    cap_y = cam.quad[2][1] + cam.thick + fsl * 1.2
    draw.text((cam.quad[3][0] + 4, cap_y), "Intel hybrid 6P + 8E   schematic, not to scale",
              font=S.font(fsl), fill=S.DIM)

    # Process labels with a leader.
    for name, cpu, x, y, labelled, block in pts:
        if not labelled:
            continue
        text = f"{name} {cpu * 100:.0f}%"
        dx, dy = LABEL_OFF.get(name, (2.4, -1.9))
        tx, ty = x + fs * dx, y + fs * dy
        draw.line([(x, y), (tx + (2 if dx < 0 else -2), ty)], fill=(235, 238, 250), width=1)
        lab(draw, tx + (-3 if dx < 0 else 3), ty, text, fsl, S.TEXT, anchor="rm" if dx < 0 else "lm")

    side_panel(img, die, w - panel, band, fs, fsl)
    if hover:
        name = hover
        b = die.by[name]
        x, y = to_screen(name, 0.30, 0.86)
        x, y = x / ss, y / ss
        tooltip(img, x, y, [f"{b.name}  {b.cpus}  {b.temp:.1f} C",
                            f"{b.freq:.1f} GHz  busy {b.busy * 100:.0f}%  ~{b.power():.1f} W",
                            f"throttles {b.throttles}  sensor: coretemp core {name[1:]}"],
                max(10, int(w / 125)))
    S.status(img, lines)
    return img


LABEL_OFF = {"browser-512": (-1.6, 2.3), "compiler-90": (1.8, 2.6), "postgres-41": (1.4, 2.4)}


def lab(draw, x, y, text, size, fill, anchor="mm", bold=False):
    f = S.font(size, S.FONT_MONO_BOLD if bold else S.FONT_MONO)
    draw.text((x + 1, y + 1), text, font=f, fill=(0, 0, 0), anchor=anchor, stroke_width=2, stroke_fill=(0, 0, 0))
    draw.text((x, y), text, font=f, fill=fill, anchor=anchor)


def tooltip(img, x, y, lines, size, left=True):
    draw = ImageDraw.Draw(img, "RGBA")
    f = S.font(size)
    wmax = max(draw.textlength(t, font=f) for t in lines)
    lh = int(size * 1.4)
    bw, bh = wmax + 16, lh * len(lines) + 10
    bx, by = (x - bw - 6, y + 10) if left else (x + 14, y + 14)
    draw.rectangle([bx, by, bx + bw, by + bh], fill=(14, 14, 22, 230), outline=(120, 126, 150, 200))
    for i, t in enumerate(lines):
        draw.text((bx + 8, by + 5 + i * lh), t, font=f, fill=S.TEXT if i == 0 else S.DIM)
    s = size * 1.1
    pts = [(x, y), (x, y + s), (x + s * 0.28, y + s * 0.74), (x + s * 0.5, y + s * 1.12),
           (x + s * 0.64, y + s * 1.04), (x + s * 0.44, y + s * 0.68), (x + s * 0.8, y + s * 0.68)]
    draw.polygon(pts, fill=(240, 240, 240), outline=(0, 0, 0))


def side_panel(img, die, x0, band, fs, fsl):
    w, h = img.size
    draw = ImageDraw.Draw(img, "RGBA")
    top = int(h * 0.06)
    bottom = h - band - int(h * 0.05)
    right = w - int(w * 0.012)
    draw.rectangle([x0, top, right, bottom], fill=(12, 10, 24, 170), outline=(70, 70, 100, 160))
    f, fb = S.font(fsl), S.font(fsl, S.FONT_MONO_BOLD)
    cw = draw.textlength("0", font=f)
    px = x0 + int(w * 0.011)
    y = top + int(fs * 0.9)
    draw.text((px, y), "SENSORS", font=S.font(fs, S.FONT_MONO_BOLD), fill=S.TEXT)
    y += int(fs * 1.8)
    # Colour bar (vertical) at the panel's right, labels to its right.
    bar_w = max(7, int(w * 0.008))
    bar_x1 = int(right - cw * 4.4)
    bar_x0 = bar_x1 - bar_w
    bar_top, bar_bot = y + 2, bottom - int(fs * 1.6)
    n = bar_bot - bar_top
    ramp = ironbow(np.linspace(1, 0, n)[:, None].repeat(bar_x1 - bar_x0, 1)).astype(np.uint8)
    img.paste(Image.fromarray(ramp), (bar_x0, bar_top))
    draw.rectangle([bar_x0 - 1, bar_top - 1, bar_x1, bar_bot], outline=(120, 126, 150))
    for temp in range(30, 106, 15):
        ty = bar_bot - (temp - T_MIN) / (T_MAX - T_MIN) * n
        draw.line([(bar_x1, ty), (bar_x1 + 4, ty)], fill=S.DIM)
        draw.text((bar_x1 + 6, ty), f"{temp}", font=f, fill=S.DIM, anchor="lm")
    ty = bar_bot - (T_THROTTLE - T_MIN) / (T_MAX - T_MIN) * n
    draw.line([(bar_x0 - 4, ty), (bar_x1 + 2, ty)], fill=(110, 240, 255), width=1)
    draw.text((bar_x0 + bar_w / 2, bar_bot + fsl * 0.5), "C", font=f, fill=S.DIM, anchor="ma")

    lh = int(fsl * 1.62)
    sq = int(fsl * 0.75)

    def row(name, temp, freq, col=S.TEXT):
        nonlocal y
        sw = ironbow(np.array([tnorm(temp)]))[0]
        draw.rectangle([px, y + (fsl - sq) // 2 + 1, px + sq, y + (fsl - sq) // 2 + 1 + sq],
                       fill=tuple(int(c) for c in sw), outline=(90, 90, 120))
        draw.text((px + cw * 1.8, y), name, font=fb, fill=col)
        draw.text((px + cw * 4.6, y), f"{temp:5.1f} C {freq:3.1f} GHz", font=f, fill=col)
        y += lh

    for b in [b for b in die.blocks if b.kind == "P"]:
        row(b.name, b.temp, b.freq, (150, 245, 255) if b.flash > 0 else S.TEXT)
    y += int(lh * 0.3)
    for c in range(2):
        es = [die.by[f"E{c * 4 + k}"] for k in range(4)]
        row(f"E{c}", max(e.temp for e in es), sum(e.freq for e in es) / 4)
    g = die.by["iGPU"]
    row("GT", g.temp, g.freq)
    y += int(lh * 0.6)
    for key, val, bold in (("package", f"{die.package():5.1f} C", True), ("RAPL pkg", f"{die.rapl:5.1f} W", True),
                           ("TjMax", f"{T_THROTTLE:5.1f} C", False)):
        draw.text((px, y), key, font=f, fill=S.DIM)
        draw.text((px + cw * 9.4, y), val, font=fb if bold else f, fill=S.TEXT if bold else S.DIM)
        y += lh
    # Package temperature and RAPL power over the last 10 s.
    hist = die.hist[-120:]
    gx0, gx1 = px, bar_x0 - cw * 2.0
    gh = min(fsl * 6.5, (bar_bot - y) * 0.42)
    if len(hist) > 2 and gh > fsl * 2.5:
        y += int(lh * 0.6)
        for title, idx, lo, hi, unit in (("package C, last 10 s", 0, T_MIN, T_MAX, "C"),
                                         ("RAPL W, last 10 s", 1, 0.0, 100.0, "W")):
            draw.text((gx0, y), title, font=f, fill=S.DIM)
            y += lh
            top, bot = y, y + gh
            draw.rectangle([gx0, top, gx1, bot], outline=(60, 60, 86), fill=(8, 6, 18, 140))
            n = len(hist)
            pts = [(gx0 + 1 + (gx1 - gx0 - 2) * i / 119, bot - 1 - (bot - top - 2) * (min(max(v[idx], lo), hi) - lo) / (hi - lo))
                   for i, v in enumerate(hist)]
            for (x0_, y0_), (x1_, y1_), v in zip(pts[:-1], pts[1:], hist[1:]):
                c = (ironbow(np.array([tnorm(v[0])]))[0] * 0.65 + np.array([110, 100, 140])) if idx == 0 else np.array([150, 200, 240])
                draw.line([(x0_, y0_), (x1_, y1_)], fill=tuple(int(k) for k in c), width=max(1, fsl // 7))
            if idx == 0:
                ty = bot - 1 - (bot - top - 2) * (T_THROTTLE - lo) / (hi - lo)
                draw.line([(gx0, ty), (gx1, ty)], fill=(110, 240, 255, 120), width=1)
            y = int(bot + lh * 0.8)


def status_lines(die, t, extra=""):
    thr = [f"{b.name} x{b.throttles}" for b in die.blocks if b.throttles]
    line = (f"ISOTOP / DIE / DEMO   192 processes | package {die.package():.1f} C | RAPL {die.rapl:.1f} W"
            f" | throttles: {', '.join(thr) if thr else 'none'} | t={t:.1f}s")
    if extra:
        line += "   " + extra
    return [line, LEGEND, KEYS]


def run(die, t_end, fps, on_frame=None, t0=0.0):
    rng = np.random.default_rng(7)
    t = t0
    dt = 1.0 / fps
    fi = 0
    while t < t_end - 1e-9:
        set_load(die, t, rng)
        die.step(STEPS_PER_FRAME)
        t += dt
        if on_frame:
            on_frame(fi, t)
        fi += 1
    return t


def warm(die):
    rng = np.random.default_rng(1)
    for _ in range(40):
        set_load(die, 0.0, rng)
        die.step(STEPS_PER_FRAME)


def extra_for(die, t):
    if t < COMPILE_START:
        return "idle: browser-512 on P1, postgres-41 on E1"
    if t < COMPILE_END:
        if any(b.flash for b in die.blocks):
            return "P2 hit TjMax: clock pulled to 4.2 GHz"
        return "make -j14: compiler-90 jobs fill the cores"
    return "build finished: the die cools"


def still():
    die = Die()
    warm(die)
    snap = {}

    def grab(fi, t):
        if abs(t - 5.0) < 1e-6 or (not snap and die.by["P2"].flash and t > 4.6):
            snap["t"] = t

    # Run to the moment of throttle plus a beat so the outline is flashing.
    t = run(die, 4.0, 12)
    rng = np.random.default_rng(5)
    while not die.by["P2"].flash and t < 6.0:
        set_load(die, t, rng)
        die.step(STEPS_PER_FRAME)
        t += 1 / 12
    for _ in range(3):
        set_load(die, t, rng)
        die.step(STEPS_PER_FRAME)
        t += 1 / 12
    img = render(die, 1600, 900, t + 0.01, live_procs(die, t, rng),
                 status_lines(die, t, extra_for(die, t)), hover="P2")
    print(S.save_still(img, "die"), "t", round(t, 2), "pkg", round(die.package(), 1), "rapl", round(die.rapl, 1))


def anim():
    die = Die()
    warm(die)
    frames = []
    rng = np.random.default_rng(9)
    fps, total = 12, 120
    t = 0.0
    for fi in range(total):
        set_load(die, t, rng)
        die.step(STEPS_PER_FRAME)
        lines = status_lines(die, t, extra_for(die, t))
        lines[1] = ("ironbow = temperature | heat = busy x (f/fmax)^3 | points = processes, size = CPU"
                    " | flash = throttle")
        frames.append(render(die, 960, 540, t, live_procs(die, t, rng), lines[:2], ss=2))
        t += 1 / fps
        if fi % 20 == 0:
            print("frame", fi, round(die.package(), 1), flush=True)
    path, size = S.save_frames(frames, "die", fps=fps)
    import stabgif
    print(stabgif.pack("die", fps=fps, colors=128, thresh=8))


def test():
    import time
    t0 = time.time()
    die = Die()
    print("gain", die.gain, time.time() - t0)
    warm(die)
    print("idle pkg", die.package(), die.rapl)
    rng = np.random.default_rng(1)
    t = 0
    for fi in range(84):
        set_load(die, t, rng)
        die.step(STEPS_PER_FRAME)
        t += 1 / 12
        if fi % 6 == 0:
            print(round(t, 2), " ".join(f"{b.name}:{b.temp:.0f}" for b in die.blocks if b.kind == "P"),
                  "E", round(die.by["E3"].temp), "rapl", round(die.rapl, 1),
                  "thr", die.by["P2"].throttles, flush=True)
    print("sim", time.time() - t0)
    img = render(die, 1600, 900, t, live_procs(die, t, rng), status_lines(die, t, extra_for(die, t)), hover="P2")
    img.save(f"{S.OUT}/die-test.png")
    print("render", time.time() - t0)


if __name__ == "__main__":
    {"still": still, "anim": anim, "test": test}[sys.argv[1] if len(sys.argv) > 1 else "test"]()
