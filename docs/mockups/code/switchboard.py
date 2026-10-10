"""Mockup of isotop's `switchboard` view: /proc/interrupts as a 1940s cord switchboard.

What is mapped (from a mocked /proc/interrupts and /proc/softirqs sample):
- Jack = one IRQ row (numeric IRQ or MSI vector), labelled with its device, grouped by device.
- Lamp brightness = log rate; flicker frequency = rate, capped at the frame rate.
- Cord = from the jack to the operator (CPU) that serviced most of that line's interrupts in
  the last interval; colour by CPU (P-cores warm, E-cores cool), thickness by rate. Cords hang
  on true catenaries for a fixed cord length. When irqbalance moves a line the plug is pulled,
  swung over and replugged at the new operator.
- Operators = CPUs, P-cores then E-cores; motion scaled by busy fraction. LOC is the ticking
  clock at each position, RES rings the operator's telephone, CAL/TLB light the green
  inter-office lamp, NMI the red one. IPI senders are unknown, so nothing links operators.
- The small board on the right is /proc/softirqs: one lamp per CPU per softirq.
Mocked: the rates, the irqbalance move at t = 3 s and the ring schedule.

usage: python3 -I switchboard.py [still|anim|both]
"""
import math
import os
import sys

sys.path.insert(0, "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto")
import colorsys  # noqa: E402

import cv2  # noqa: E402
import numpy as np  # noqa: E402
from PIL import ImageDraw  # noqa: E402
from scipy.optimize import brentq  # noqa: E402

import isostyle as S  # noqa: E402
import mockkit as K  # noqa: E402

NCPU = 16
FPS = 12
# Board geometry (design px at 1600 x 900). Oblique camera: world depth d (toward the viewer)
# shifts a point left and down: sx = x - 0.2 d, sy = -z + 0.486 d.
BX0, BX1 = 62, 1216           # main board face
TOP = 64                      # cornice top
ROW_Y = [124, 206, 288, 370]  # lamp y per jack row; jack at +24, label strip above at -27
COLX0, PITCH = 112, 92.0
SHELF_Y = 482                 # back edge of the keyshelf (d = 0)
SHELF_D = 74
OP_D = 150
OP_Y = 548                    # operator shoulder line (screen)
OPX0, OPP = 96, 69.6          # operator x (world) per position
SQX0, SQX1, SQTOP = 1252, 1578, 64   # softirq board

LINES = [
    # row 1
    ("nvme0q0", 24, 0), ("nvme0q1", 1460, 0), ("nvme0q2", 980, 1), ("nvme0q3", 2210, 2),
    ("nvme0q4", 640, 3), ("nvme0q5", 1320, 4), ("nvme0q6", 410, 5), ("nvme0q7", 760, 6),
    ("nvme0q8", 520, 7), ("ahci", 85, 11), ("xhci_hcd", 1530, 4), ("i915", 2410, 15),
    # row 2
    ("eth0-rx-0", 3912, 0), ("eth0-rx-1", 2850, 2), ("eth0-rx-2", 3104, 3), ("eth0-rx-3", 2240, 5),
    ("eth0-tx-0", 820, 6), ("eth0-tx-1", 640, 7), ("eth0-tx-2", 710, 3), ("eth0-tx-3", 450, 10),
    ("hda_intel", 960, 11), ("mei_me", 14, 12), ("intel_ish", 40, 14), ("i801_smbus", 3, 0),
    # row 3
    ("iwlwifi", 380, 1), ("iwlwifi:q1", 1240, 8), ("iwlwifi:q2", 860, 2), ("iwlwifi:q3", 1430, 5),
    ("iwlwifi:q4", 520, 9), ("iwlwifi:q5", 960, 7), ("iwlwifi:q6", 300, 10), ("iwlwifi:q7", 610, 12),
    ("thunderbolt", 6, 13), ("ucsi_acpi", 2, 0), ("pinctrl", 11, 14), ("dw_dmac", 0.4, 15),
    # row 4
    ("timer", 1, 0), ("rtc0", 1, 0), ("acpi", 22, 0), ("i2c_dw.0", 118, 9),
    ("hid-i2c", 120, 9), ("idma64.0", 5, 8), ("idma64.1", 0.2, 8), ("dmar0", 0.1, 0),
    ("dmar1", 0.1, 0), ("aerdrv", 0.0, 0), ("PCIe PME", 0.0, 0), ("pciehp", 0.3, 0),
]
TOTAL = 41203.0
_s = sum(r for _, r, _ in LINES)
LINES = [(n, r * TOTAL / _s, c) for n, r, c in LINES]
GROUPS = [(0, 9), (9, 10), (10, 11), (11, 12), (12, 20), (20, 24), (24, 32), (32, 36), (36, 48)]
MOVE = dict(line=14, frm=3, to=9, t0=3.0, lift=0.35, swing=1.05, seat=0.25)
SOFT = ["HI", "TIMER", "NET_TX", "NET_RX", "BLOCK", "IRQ_POLL", "TASKLET", "SCHED", "HRTIMER", "RCU"]
BUSY = [0.64, 0.41, 0.72, 0.38, 0.55, 0.47, 0.33, 0.29, 0.22, 0.31, 0.18, 0.26, 0.35, 0.12, 0.09, 0.15]
RINGS = [(2, 0.3), (9, 1.1), (5, 1.8), (12, 2.6), (0, 3.4), (7, 4.0), (3, 4.9), (14, 5.5),
         (9, 6.1), (1, 6.9), (11, 7.6), (6, 8.3), (4, 3.55)]


def cpu_colour(c):
    if c < 8:   # P-cores: warm cloth dyes
        h = [0.0, 0.035, 0.07, 0.105, 0.135, 0.955, 0.02, 0.09][c]
        l = [0.52, 0.60, 0.50, 0.62, 0.55, 0.50, 0.40, 0.42][c]
        sat = 0.72
    else:       # E-cores: cool
        h = [0.47, 0.42, 0.53, 0.38, 0.57, 0.50, 0.33, 0.62][c - 8]
        l = [0.50, 0.45, 0.58, 0.52, 0.55, 0.40, 0.46, 0.60][c - 8]
        sat = 0.55
    r, g, b = colorsys.hls_to_rgb(h, l, sat)
    return np.array([r, g, b]) * 255


def scr(x, z, d):
    return np.array([x - 0.2 * d, -z + 0.486 * d])


def catenary(p0, p1, slack, n=60):
    """True catenary through two world points (x, d, z) for cord length slack * chord."""
    p0, p1 = np.array(p0, float), np.array(p1, float)
    hv = p1[:2] - p0[:2]
    h = max(float(np.linalg.norm(hv)), 1e-3)
    v = p1[2] - p0[2]
    L = math.hypot(h, v) * slack
    k = math.sqrt(max(L * L - v * v, 1e-6))
    a = brentq(lambda a: 2 * a * math.sinh(h / (2 * a)) - k, 0.02 * h, 1e6 * h)
    u0 = h / 2 - a * math.atanh(np.clip(v / L, -0.999999, 0.999999))
    c = p0[2] - a * math.cosh((0 - u0) / a)
    u = np.linspace(0, h, n)
    z = a * np.cosh((u - u0) / a) + c
    xy = p0[:2] + np.outer(u / h, hv)
    return np.stack([xy[:, 0], xy[:, 1], z], 1)


def jack_pos(i):
    row, col = divmod(i, 12)
    return COLX0 + col * PITCH, ROW_Y[row]


def hole(cpu, slot):
    """Plug seat on the keyshelf for an operator position (world x, d, z)."""
    x = OPX0 + cpu * OPP - 14 + slot * 8.5
    return (x, 34.0, -SHELF_Y)


class Board:
    def __init__(self):
        rng = np.random.default_rng(4)
        self.assign = [c for _, _, c in LINES]
        self.slots = {}
        counts = [0] * NCPU
        for i, c in enumerate(self.assign):
            self.slots[i] = counts[c]
            counts[c] += 1
        self.slots[("new", MOVE["line"])] = counts[MOVE["to"]]
        # Lamp flicker: Poisson events at min(rate, fps) per second (rate is noted, not shown).
        self.events = []
        for _, r, _ in LINES:
            lam = min(r, FPS * 0.9)
            n = rng.poisson(lam * 14)
            self.events.append(np.sort(rng.uniform(-2, 12, n)))
        self.slack = rng.uniform(1.02, 1.07, len(LINES))
        # softirq rates per CPU
        sq = np.zeros((len(SOFT), NCPU))
        sq[SOFT.index("TIMER")] = rng.uniform(180, 260, NCPU)
        sq[SOFT.index("HRTIMER")] = rng.uniform(2, 40, NCPU)
        sq[SOFT.index("RCU")] = rng.uniform(60, 320, NCPU)
        sq[SOFT.index("SCHED")] = rng.uniform(40, 520, NCPU) * np.array(BUSY) * 2
        sq[SOFT.index("TASKLET")][[2, 8, 9, 10, 11, 12]] = rng.uniform(40, 300, 6)
        sq[SOFT.index("HI")][[4]] = 30
        for i, (n, r, c) in enumerate(LINES):
            if n.startswith("eth0-rx"):
                sq[SOFT.index("NET_RX"), c] += r * 0.8
            if n.startswith("eth0-tx"):
                sq[SOFT.index("NET_TX"), c] += r * 0.5
            if n.startswith("nvme"):
                sq[SOFT.index("BLOCK"), c] += r * 0.9
            if n.startswith("iwlwifi"):
                sq[SOFT.index("NET_RX"), c] += r * 0.4
        self.sq = sq
        self.sq_events = [[np.sort(rng.uniform(-2, 12, rng.poisson(min(sq[a, b], FPS * 0.9) * 14)))
                           for b in range(NCPU)] for a in range(len(SOFT))]

    def cord_state(self, i, t):
        """World endpoints for line i at time t, its colour, and whether it is in the air."""
        jx, jy = jack_pos(i)
        jack = (jx, 0.0, -(jy + 24))
        c = self.assign[i]
        if i != MOVE["line"]:
            return jack, hole(c, self.slots[i]), cpu_colour(c), 0.0
        m = MOVE
        a = hole(m["frm"], self.slots[i])
        b = hole(m["to"], self.slots[("new", i)])
        t1 = m["t0"] + m["lift"]
        t2 = t1 + m["swing"]
        t3 = t2 + m["seat"]
        if t < m["t0"]:
            return jack, a, cpu_colour(m["frm"]), 0.0
        if t >= t3:
            return jack, b, cpu_colour(m["to"]), 0.0
        if t < t1:
            f = (t - m["t0"]) / m["lift"]
            lift = 70 * (1 - (1 - f) ** 2)
            p = np.array(a) + (0, 8 * f, lift)
            return jack, tuple(p), cpu_colour(m["frm"]), lift
        if t < t2:
            f = (t - t1) / m["swing"]
            f = f * f * (3 - 2 * f)
            p = np.array(a) * (1 - f) + np.array(b) * f + (0, 8, 70 + 60 * math.sin(math.pi * f))
            col = cpu_colour(m["frm"]) * (1 - f) + cpu_colour(m["to"]) * f
            return jack, tuple(p), col, 70 + 60 * math.sin(math.pi * f)
        f = (t - t2) / m["seat"]
        p = np.array(b) + (0, 8 * (1 - f), 70 * (1 - f) ** 1.5)
        return jack, tuple(p), cpu_colour(m["to"]), 70 * (1 - f)

    def lamp(self, i, t):
        r = LINES[i][1]
        b = min(math.log10(r + 1) / math.log10(8000), 1.0)
        ev = self.events[i]
        rec = ev[(ev <= t) & (ev > t - 0.5)]
        flick = float(np.exp(-(t - rec) / 0.05).max()) if rec.size else 0.0
        if i == MOVE["line"] and MOVE["t0"] <= t < MOVE["t0"] + 1.65:
            flick = 0.5 + 0.5 * math.cos(t * 30)   # line lamp flutters while unplugged
        return b * (0.42 + 0.58 * flick) if r > 0.5 else b * flick * 0.8

    def soft_lamp(self, a, b, t):
        r = self.sq[a, b]
        if r < 0.5:
            return 0.0
        br = min(math.log10(r + 1) / math.log10(5000), 1.0)
        ev = self.sq_events[a][b]
        rec = ev[(ev <= t) & (ev > t - 0.5)]
        flick = float(np.exp(-(t - rec) / 0.05).max()) if rec.size else 0.0
        return br * (0.4 + 0.6 * flick)


class Renderer:
    def __init__(self, board, w, h, ss=2):
        self.bd, self.w, self.h, self.ss = board, w, h, ss
        self.s = w / 1600.0
        self.k = self.s * ss
        self.base = self.make_base()

    def P(self, p):
        return (p[0] * self.k, p[1] * self.k)

    def wood(self, shape, rng, tone, scale=1.0, stretch=10):
        H, W = shape
        n = K.value_noise(256, 6, rng, ((1, 1), (4, 0.4)))
        g = cv2.resize(n, (max(W // stretch, 2), H), interpolation=cv2.INTER_CUBIC)
        g = cv2.resize(g, (W, H), interpolation=cv2.INTER_CUBIC)
        fine = rng.standard_normal((H, max(W // 40, 2))).astype(np.float32)
        fine = cv2.resize(cv2.GaussianBlur(fine, (0, 0), 1.2), (W, H), interpolation=cv2.INTER_LINEAR)
        grain = 0.5 + 0.5 * np.sin(g * 5.0 + fine * 0.8)
        tone = np.array(tone, np.float32)
        return tone * (0.78 + 0.32 * grain[..., None] * scale)

    def quad(self, canvas, pts, colour=None, tex=None, alpha=1.0):
        pts = np.asarray(pts, float) * self.k
        if tex is None:
            K.blend_poly(canvas, pts, colour, alpha)
            return
        m = K.mask_poly(canvas.shape, pts)[..., None] * alpha
        canvas[:] = canvas * (1 - m) + tex * m

    def make_base(self):
        W, H, k = self.w * self.ss, self.h * self.ss, self.k
        rng = np.random.default_rng(12)
        sky = S.sky(self.w, self.h, glow=(160, 92, 70), base=(13, 9, 14), centre=(0.16, 0.18))
        c = cv2.resize(sky, (W, H), interpolation=cv2.INTER_LINEAR)
        # Floor: dark boards receding.
        fl = self.wood((H, W), rng, (46, 30, 22), 0.8, stretch=3)
        self.quad(c, [(0, 600), (1600, 600), (1600, 900), (0, 900)], tex=fl, alpha=0.75)
        lay = K.Layer(W, H)
        for j in range(-8, 40):
            x0 = j * 52
            p0, p1 = scr(x0, -600, 0), scr(x0, -600 - 0, 640)
            lay.line(self.P((x0 + 30, 600)), self.P((x0 + 30 - 0.2 * 620, 600 + 0.486 * 620)), (20, 13, 10), 1.2 * k)
        lay.onto(c, 0.5)
        # Main board: right side face and cornice top (receding), then the face.
        oak = self.wood((H, W), rng, (118, 70, 38))
        dark_oak = self.wood((H, W), rng, (74, 44, 26))
        for (x0, x1, top) in ((BX0, BX1, TOP), (SQX0, SQX1, SQTOP)):
            depth = 46
            back = 0.2 * depth, -0.486 * depth
            self.quad(c, [(x1, top), (x1 + back[0], top + back[1]), (x1 + back[0], SHELF_Y + back[1] + 60),
                          (x1, SHELF_Y + 60)], tex=dark_oak * 0.75)
            self.quad(c, [(x0 - 8, top), (x1 + 8, top), (x1 + 8 + back[0], top + back[1]),
                          (x0 - 8 + back[0], top + back[1])], tex=oak * 1.15)
        self.quad(c, [(BX0 - 8, TOP), (BX1 + 8, TOP), (BX1 + 8, SHELF_Y + 4), (BX0 - 8, SHELF_Y + 4)], tex=oak)
        # Cornice moulding.
        self.quad(c, [(BX0 - 8, TOP), (BX1 + 8, TOP), (BX1 + 8, TOP + 14), (BX0 - 8, TOP + 14)], tex=oak * 1.25)
        self.quad(c, [(BX0 - 8, TOP + 14), (BX1 + 8, TOP + 14), (BX1 + 8, TOP + 17), (BX0 - 8, TOP + 17)], (40, 24, 14))
        # Jack strips (bakelite) and ivory designation strips.
        for r, y in enumerate(ROW_Y):
            self.quad(c, [(BX0 + 14, y - 19), (BX1 - 14, y - 19), (BX1 - 14, y + 36), (BX0 + 14, y + 36)], (24, 17, 14))
            self.quad(c, [(BX0 + 14, y - 36), (BX1 - 14, y - 36), (BX1 - 14, y - 19), (BX0 + 14, y - 19)], (208, 192, 152))
            self.quad(c, [(BX0 + 14, y + 36), (BX1 - 14, y + 36), (BX1 - 14, y + 39), (BX0 + 14, y + 39)], (52, 32, 20))
        # Lower rail: clocks and inter-office lamps, one set per position.
        self.quad(c, [(BX0 + 14, 432), (BX1 - 14, 432), (BX1 - 14, 474), (BX0 + 14, 474)], (30, 21, 16))
        # Keyshelf: top surface (receding toward the viewer) and front face.
        a, b = scr(BX0 - 8, -SHELF_Y, 0), scr(BX1 + 8, -SHELF_Y, 0)
        cc, dd = scr(BX1 + 8, -SHELF_Y, SHELF_D), scr(BX0 - 8, -SHELF_Y, SHELF_D)
        self.quad(c, [a, b, cc, dd], tex=oak * 1.18)
        self.quad(c, [dd, cc, cc + (0, 22), dd + (0, 22)], tex=dark_oak)
        self.quad(c, [dd + (0, 22), cc + (0, 22), cc + (0, 25), dd + (0, 25)], (20, 12, 8))
        # Cabinet below the shelf (in shadow).
        self.quad(c, [dd + (0, 25), cc + (0, 25), cc + (0, 150), dd + (0, 150)], tex=dark_oak * 0.55)
        for xx in np.arange(BX0 + 60, BX1, 140):
            q = scr(xx, -SHELF_Y, SHELF_D)
            self.quad(c, [q + (0, 36), q + (100, 36), q + (100, 140), q + (0, 140)], tex=dark_oak * 0.45)
        # Softirq board face.
        self.quad(c, [(SQX0, SQTOP), (SQX1, SQTOP), (SQX1, 560), (SQX0, 560)], tex=oak)
        self.quad(c, [(SQX0, SQTOP), (SQX1, SQTOP), (SQX1, SQTOP + 12), (SQX0, SQTOP + 12)], tex=oak * 1.25)
        self.quad(c, [(SQX0 + 12, 110), (SQX1 - 12, 110), (SQX1 - 12, 540), (SQX0 + 12, 540)], (24, 17, 14))
        self.quad(c, [(SQX0 + 12, 80), (SQX1 - 12, 80), (SQX1 - 12, 100), (SQX0 + 12, 100)], (208, 192, 152))
        # Fixed hardware: jacks, lamp lenses (off), group dividers, softirq lamps (off).
        lay = K.Layer(W, H)
        for i in range(len(LINES)):
            x, y = jack_pos(i)
            lay.circle(self.P((x, y)), 7.2 * k, (58, 34, 18))
            lay.circle(self.P((x, y)), 5.4 * k, (92, 58, 30))
            lay.circle(self.P((x - 1.2, y - 1.4)), 1.6 * k, (150, 110, 70))
            lay.circle(self.P((x, y + 24)), 6.6 * k, (176, 136, 70))
            lay.circle(self.P((x, y + 24)), 4.6 * k, (120, 88, 40))
            lay.circle(self.P((x, y + 24)), 3.2 * k, (6, 4, 3))
        for a0, a1 in GROUPS[1:]:
            row, col = divmod(a0, 12)
            if col == 0:
                continue
            x = COLX0 + (col - 0.5) * PITCH
            y = ROW_Y[row]
            lay.line(self.P((x, y - 36)), self.P((x, y + 36)), (150, 112, 60), 1.5 * k)
        for a in range(len(SOFT)):
            for b in range(NCPU):
                x, y = self.sq_pos(a, b)
                lay.circle(self.P((x, y)), 5.0 * k, (58, 34, 18))
                lay.circle(self.P((x, y)), 3.7 * k, (90, 56, 30))
        for cp in range(NCPU):
            x = OPX0 + cp * OPP
            lay.circle(self.P((x - 8, 453)), 11 * k, (214, 204, 176))
            lay.circle(self.P((x - 8, 453)), 11 * k, (90, 66, 40), 1.4 * k)
            for tk in range(12):
                ang = tk * math.pi / 6
                lay.line(self.P((x - 8 + 8 * math.cos(ang), 453 + 8 * math.sin(ang))),
                         self.P((x - 8 + 10 * math.cos(ang), 453 + 10 * math.sin(ang))), (60, 50, 40), 0.8 * k)
            lay.circle(self.P((x + 12, 446)), 3.6 * k, (30, 46, 30))
            lay.circle(self.P((x + 12, 460)), 3.6 * k, (54, 22, 18))
            # Key levers on the shelf.
            for kk in range(3):
                p = scr(x - 18 + kk * 12, -SHELF_Y, 58)
                lay.circle(self.P(p), 2.6 * k, (226, 214, 190) if kk != 1 else (40, 30, 24))
            for sl in range(5):
                hp = hole(cp, sl)
                p = scr(hp[0], hp[2], hp[1])
                lay.ellipse(self.P(p), (3.0 * k, 1.6 * k), 0, (14, 9, 6))
        lay.onto(c)
        yy, xx = np.mgrid[0:H, 0:W].astype(np.float32) / k
        pool = np.exp(-(((xx - 640) / 620) ** 2 + ((yy - 300) / 330) ** 2))
        c *= (0.78 + 0.42 * pool)[..., None]
        return c

    def sq_pos(self, a, b):
        return SQX0 + 84 + b * 14.6, 134 + a * 40.5

    # ------------------------------------------------------------------ frame
    def frame(self, t):
        bd, k = self.bd, self.k
        c = self.base.copy()
        W, H = c.shape[1], c.shape[0]
        glow = np.zeros_like(c)
        lay = K.Layer(W, H)
        amber = np.array([255, 178, 82], np.float32)
        for i in range(len(LINES)):
            x, y = jack_pos(i)
            v = bd.lamp(i, t)
            if v > 0.02:
                col = np.array([92, 58, 30]) * (1 - v) + np.array([255, 214, 140]) * v
                lay.circle(self.P((x, y)), 5.4 * k, tuple(col))
                S.glow(glow, x * k, y * k, 11 * k, amber, 0.75 * v ** 1.3)
        for a in range(len(SOFT)):
            for b in range(NCPU):
                v = bd.soft_lamp(a, b, t)
                if v > 0.02:
                    x, y = self.sq_pos(a, b)
                    col = np.array([90, 56, 30]) * (1 - v) + np.array([255, 214, 140]) * v
                    lay.circle(self.P((x, y)), 3.7 * k, tuple(col))
                    S.glow(glow, x * k, y * k, 6 * k, amber, 0.5 * v ** 1.3)
        # Clocks (LOC ticks), inter-office (CAL/TLB) and NMI lamps.
        for cp in range(NCPU):
            x = OPX0 + cp * OPP
            loc = 250 + 750 * BUSY[cp]
            ang = -math.pi / 2 + (t * loc / 250.0 + cp) * 2 * math.pi / 12
            ang = math.floor(ang / (math.pi / 30)) * (math.pi / 30)   # ticking
            lay.line(self.P((x - 8, 453)), self.P((x - 8 + 8 * math.cos(ang), 453 + 8 * math.sin(ang))), (40, 20, 14), 1.2 * k)
            lay.line(self.P((x - 8, 453)), self.P((x - 8 + 5 * math.cos(ang / 12 + 1), 453 + 5 * math.sin(ang / 12 + 1))), (40, 30, 24), 1.5 * k)
            cal = 0.5 + 0.5 * math.sin(t * (3 + cp * 0.7) + cp)
            if cal > 0.55:
                lay.circle(self.P((x + 12, 446)), 3.6 * k, (120, 240, 140))
                S.glow(glow, (x + 12) * k, 446 * k, 6 * k, (90, 255, 120), 0.35 * cal)
        c += glow
        lay.onto(c)
        # Telephones on the shelf (before the cords, which drape over).
        lay = K.Layer(W, H)
        rings = []
        for cp in range(NCPU):
            x = OPX0 + cp * OPP + 30
            p = scr(x, -SHELF_Y, 50)
            ringing = 0.0
            for rc, t0 in RINGS:
                if rc == cp and t0 <= t < t0 + 0.9:
                    ringing = 1.0
            jit = (math.sin(t * 95 + cp) * 1.6 if ringing else 0.0)
            px, py = p[0] + jit, p[1]
            body = (12, 10, 9)
            lay.poly(np.array([(px - 11, py + 6), (px + 11, py + 6), (px + 8, py - 5), (px - 8, py - 5)]) * k, body)
            lay.line(self.P((px - 7, py - 5)), self.P((px + 7, py - 5)), (70, 64, 60), 0.9 * k)
            hl = -8 - (3.5 + abs(jit) if ringing else 0)
            lay.line(self.P((px - 11, py + hl)), self.P((px + 11, py + hl - jit * 0.6)), body, 4.2 * k)
            lay.circle(self.P((px - 11, py + hl + 1)), 3.4 * k, body)
            lay.circle(self.P((px + 11, py + hl + 1 - jit * 0.6)), 3.4 * k, body)
            lay.line(self.P((px - 9, py + hl - 1.6)), self.P((px + 9, py + hl - 1.6 - jit * 0.6)), (90, 84, 80), 0.8 * k)
            lay.circle(self.P((px, py + 1.5)), 3.0 * k, (210, 200, 170))
            lay.circle(self.P((px, py + 1.5)), 1.0 * k, (40, 34, 30))
            if ringing:
                rings.append((px, py - 4))
        lay.onto(c)
        # Cords: catenaries from jack to the operator's plug seat.
        lay = K.Layer(W, H)
        plugs, shadows = [], []
        order = sorted(range(len(LINES)), key=lambda i: -jack_pos(i)[1])
        for i in order:
            jack, seat, col, air = bd.cord_state(i, t)
            p0 = (jack[0], 4.0, jack[2])
            chord = float(np.linalg.norm(np.array(seat) - np.array(p0)))
            slack = 1.0 + (bd.slack[i] - 1.0) * 260.0 / max(chord, 60.0) + (0.04 if air else 0.0)
            for _ in range(12):
                pts = catenary(p0, seat, slack)
                if pts[:, 2].min() >= seat[2] - 3 or slack < 1.0015:
                    break
                slack = 1.0 + (slack - 1.0) * 0.6
            sp = np.stack([pts[:, 0] - 0.2 * pts[:, 1], -pts[:, 2] + 0.486 * pts[:, 1]], 1)
            rate = LINES[i][1]
            wdt = 1.3 + 0.75 * math.log10(rate + 1)
            col = np.asarray(col, float)
            lay.polyline(sp * k, tuple(col * 0.35), (wdt + 1.4) * k)
            lay.polyline(sp * k, tuple(col), wdt * k)
            lay.polyline((sp + (-0.25 * wdt, -0.3 * wdt)) * k, tuple(np.clip(col * 1.25 + 40, 0, 255)), max(0.6, wdt * 0.3) * k)
            plugs.append((jack, sp[0], sp[-1], air, col, wdt))
            shadows.append((sp, wdt))
        sh = K.Layer(W, H)
        for sp, wdt in shadows:
            up = sp[sp[:, 1] < SHELF_Y - 4]
            if len(up) > 1:
                sh.polyline((up + (7, 5)) * k, (0, 0, 0), (wdt + 2) * k)
        sh.rgb = cv2.GaussianBlur(sh.rgb, (0, 0), 2.0 * k)
        sh.a = cv2.GaussianBlur(sh.a, (0, 0), 2.0 * k)
        sh.onto(c, 0.45)
        for jack, j0, s1, air, col, wdt in plugs:
            # Plug sleeve in the jack, plug shell at the seat (lifted when in the air).
            lay.circle(self.P(j0), 5.2 * k, (186, 146, 76))
            lay.circle(self.P(j0), 3.0 * k, (230, 196, 120))
            if air:
                lay.line(self.P(s1), self.P(s1 + (0, 9)), (186, 146, 76), 4.2 * k)
                lay.circle(self.P(s1 + (0, 10)), 1.6 * k, (230, 196, 120))
            else:
                lay.ellipse(self.P(s1), (3.6 * k, 2.2 * k), 0, (186, 146, 76))
        lay.onto(c)
        # The swing path of a plug being moved by irqbalance (a dotted guide, fades after).
        m = MOVE
        tend = m["t0"] + m["lift"] + m["swing"] + m["seat"]
        if m["t0"] <= t < tend + 0.8:
            fade = 1.0 if t < tend else 1.0 - (t - tend) / 0.8
            g = K.Layer(W, H)
            for tt in np.linspace(m["t0"], tend, 46):
                if tt > t:
                    break
                _, p3, _, _ = bd.cord_state(m["line"], tt)
                q = scr(p3[0], p3[2], p3[1]) + (0, 10)
                g.circle(self.P(q), 1.6 * k, (255, 236, 190))
            g.onto(c, 0.7 * fade)
            if t < tend:
                _, p3, _, _ = bd.cord_state(m["line"], t)
                q = scr(p3[0], p3[2], p3[1]) + (0, 10)
                S.glow(c, q[0] * k, q[1] * k, 10 * k, (255, 220, 160), 0.6)
        # Ring marks.
        lay = K.Layer(W, H)
        for px, py in rings:
            for rr in (17, 23):
                lay.ellipse(self.P((px, py - 4)), (rr * k, rr * 0.85 * k), 0, (255, 240, 200), 1.6 * k, 150, 210)
                lay.ellipse(self.P((px, py - 4)), (rr * k, rr * 0.85 * k), 0, (255, 240, 200), 1.6 * k, 330, 390)
        lay.onto(c, 0.9)
        # Operators, seated in front of the shelf, seen from behind.
        for cp in range(NCPU):
            self.operator(c, cp, t)
        img = K.downsample(c, self.w, self.h)
        self.text(img, t, rings)
        return img

    def operator(self, c, cp, t):
        k = self.k
        W, H = c.shape[1], c.shape[0]
        rng = np.random.default_rng(100 + cp)
        x = OPX0 + cp * OPP
        bx, by = x - 20, OP_Y
        busy = BUSY[cp]
        sway = math.sin(t * (1.3 + busy * 3) + cp) * (1.0 + 5 * busy)
        dress = [(52, 58, 86), (86, 38, 44), (40, 70, 60), (70, 66, 74), (90, 70, 50), (44, 48, 70)][cp % 6]
        hair = [(40, 26, 18), (24, 18, 16), (92, 52, 28), (60, 38, 24)][(cp * 3) % 4]
        lay = K.Layer(W, H)
        # Arms (reaching to the shelf / board by busy fraction).
        sh_l, sh_r = (bx - 17 + sway * 0.3, by + 16), (bx + 17 + sway * 0.3, by + 16)
        reach = 0.5 + 0.5 * math.sin(t * (2 + 4 * busy) + cp * 1.7)
        hand_r = (bx + 19 + 5 * reach * busy, by - 20 - 26 * busy * reach)
        hand_l = (bx - 19 - 3 * busy, by - 12)
        for sh, hd, el in ((sh_l, hand_l, (bx - 27, by + 34)), (sh_r, hand_r, (bx + 27, by + 34 - 8 * busy * reach))):
            lay.line(self.P(sh), self.P(el), tuple(np.array(dress) * 0.85), 8 * k)
            lay.line(self.P(el), self.P(hd), tuple(np.array(dress) * 0.78), 7 * k)
            lay.circle(self.P(el), 3.8 * k, tuple(np.array(dress) * 0.82))
            lay.circle(self.P(hd), 3.6 * k, (214, 168, 136))
        # Torso.
        tor = np.array([(bx - 21, by + 70), (bx + 21, by + 70), (bx + 20, by + 14), (bx + 12, by + 6),
                        (bx - 12, by + 6), (bx - 20, by + 14)]) + (sway * 0.3, 0)
        lay.poly(tor * k, dress)
        lay.line(self.P((bx + sway * 0.3, by + 10)), self.P((bx + sway * 0.3, by + 64)), tuple(np.array(dress) * 0.7), 1.0 * k)
        # Neck and head (1940s rolled hair), headset band and receiver.
        hx, hy = bx + sway * 0.45, by - 6
        lay.line(self.P((hx, hy + 8)), self.P((bx + sway * 0.3, by + 8)), (190, 146, 118), 7 * k)
        lay.circle(self.P((hx, hy)), 11.5 * k, hair)
        lay.circle(self.P((hx - 7, hy - 8)), 5.5 * k, tuple(np.array(hair) * 1.15))
        lay.circle(self.P((hx + 7, hy - 8)), 5.5 * k, tuple(np.array(hair) * 1.15))
        lay.circle(self.P((hx, hy + 9)), 6.5 * k, tuple(np.array(hair) * 1.25))
        lay.ellipse(self.P((hx, hy - 1)), (12.5 * k, 13.5 * k), 0, (34, 34, 38), 1.8 * k, 185, 355)
        lay.circle(self.P((hx + 12, hy + 1)), 4.2 * k, (28, 28, 32))
        lay.circle(self.P((hx + 12.5, hy + 0.5)), 1.4 * k, (120, 120, 130))
        lay.line(self.P((hx + 11, hy + 5)), self.P((hx + 6, hy + 20)), (30, 30, 34), 1.0 * k)
        # Chair back (closest to us).
        cb = np.array([(bx - 16, by + 92), (bx + 16, by + 92), (bx + 17, by + 46), (bx + 11, by + 38),
                       (bx - 11, by + 38), (bx - 17, by + 46)])
        lay.poly(cb * k, (86, 50, 28))
        lay.poly((cb * 0.86 + np.array([bx, by + 60]) * 0.14) * k, (104, 62, 34))
        lay.line(self.P((bx - 14, by + 92)), self.P((bx - 16, by + 140)), (58, 34, 20), 3.5 * k)
        lay.line(self.P((bx + 14, by + 92)), self.P((bx + 16, by + 140)), (58, 34, 20), 3.5 * k)
        # Shadow on the floor.
        sh = K.Layer(W, H)
        sh.ellipse(self.P((bx + 4, by + 142)), (26 * k, 7 * k), 0, (0, 0, 0))
        sh.onto(c, 0.35)
        lay.onto(c)

    def text(self, img, t, rings):
        bd, s = self.bd, self.s
        d = ImageDraw.Draw(img)

        def F(px, bold=False):
            return S.font(max(7, round(px * s)), S.FONT_MONO_BOLD if bold else S.FONT_MONO)

        def T(x, y, text, px=12, fill=S.TEXT, anchor="la", bold=False, shadow=True):
            f = F(px, bold)
            if shadow:
                d.text((x * s + 1, y * s + 1), text, font=f, fill=(0, 0, 0), anchor=anchor)
            d.text((x * s, y * s), text, font=f, fill=fill, anchor=anchor)

        # Designation strips (printed, so dark ink on ivory, no shadow).
        for i, (name, r, cpu) in enumerate(LINES):
            x, y = jack_pos(i)
            T(x, y - 27, name, 11, (40, 28, 18), "mm", shadow=False)
        T((BX0 + BX1) / 2, TOP + 8, "IRQ LINES  /proc/interrupts", 11, (52, 30, 16), "mm", bold=True, shadow=False)
        T((SQX0 + SQX1) / 2, 90, "SOFTIRQ  /proc/softirqs", 11, (40, 28, 18), "mm", bold=True, shadow=False)
        for a, nm in enumerate(SOFT):
            x, y = self.sq_pos(a, 0)
            T(SQX0 + 22, y, nm, 10, (214, 196, 160), "lm", shadow=False)
        for b in range(NCPU) if s > 0.9 else (0, 4, 8, 12):
            x, _ = self.sq_pos(0, b)
            T(x, 122, f"{b}", 8, (190, 170, 130), "mm", shadow=False)
        T(SQX0 + 84 + 3.5 * 14.6, 528, "P cores", 9, (190, 170, 130), "mm", shadow=False)
        T(SQX0 + 84 + 11.5 * 14.6, 528, "E cores", 9, (190, 170, 130), "mm", shadow=False)
        # Operator name plates.
        for cp in range(NCPU):
            x = OPX0 + cp * OPP
            bx, by = x - 20, OP_Y
            T(bx + 4, by + 160, f"cpu{cp} {'P' if cp < 8 else 'E'}", 11, S.TEXT, "mm")
            T(bx + 4, by + 175, f"{BUSY[cp] * 100:.0f}% busy", 10, S.DIM, "mm")
        # Inspector for the moving line.
        m = MOVE
        i = m["line"]
        x0, y0 = 1252, 600
        dd = ImageDraw.Draw(img)
        dd.rectangle([x0 * s, y0 * s, 1578 * s, 790 * s], fill=(14, 12, 16), outline=(90, 70, 50))
        name, rate, _ = LINES[i]
        cur = m["frm"] if t < m["t0"] + m["lift"] + m["swing"] else m["to"]
        T(x0 + 12, y0 + 12, f"{name}   IRQ 147  PCI-MSIX", 13, (250, 214, 150), bold=True)
        T(x0 + 12, y0 + 38, f"{rate:,.0f} irq/s  lamp flicker capped at {FPS} Hz", 11, S.DIM)
        T(x0 + 12, y0 + 58, f"serviced by cpu{cur} ({'P' if cur < 8 else 'E'})", 11, S.DIM)
        T(x0 + 12, y0 + 78, f"affinity 0-15   effective {cur}", 11, S.DIM)
        if t >= m["t0"]:
            T(x0 + 12, y0 + 106, f"irqbalance: cpu{m['frm']} -> cpu{m['to']}", 12, (250, 214, 150))
            t3 = m["t0"] + m["lift"] + m["swing"] + m["seat"]
            if t < t3:
                T(x0 + 12, y0 + 126, "  cord unplugged, swinging over", 11, S.DIM)
            else:
                T(x0 + 12, y0 + 126, f"  replugged {t - t3:.1f} s ago", 11, S.DIM)
        else:
            T(x0 + 12, y0 + 106, "irqbalance: watching", 12, S.DIM)
        rr = sum(1 for rc, t0 in RINGS if t0 <= t < t0 + 0.9)
        T(x0 + 12, y0 + 156, f"phones ringing {rr}  (RES IPIs)", 11, S.DIM)
        lines = [
            "ISOTOP / SWITCHBOARD / DEMO  48 lines  41,203 irq/s  RES 2,114/s   16 operators: 8 P-cores, "
            "8 E-cores",
            "jack = IRQ line | lamp = rate (log) | cord = CPU servicing the line, colour by CPU, thickness "
            "by rate | operator = CPU, motion by busy",
            "phone rings = RES IPI | clock = LOC ticks | green lamp = CAL/TLB | red lamp = NMI | right: "
            "softirqs per CPU | IPI senders unknown, no operator links",
        ]
        S.status(img, lines, size=max(9, round(13 * s)))


def pack(frames, name, fps):
    import gifpack
    S.save_frames(frames, name, fps=fps)
    for width, colors in ((720, 128), (720, 96), (720, 64), (640, 96), (640, 64), (560, 64)):
        path, size = gifpack.pack(name, fps=fps, width=width, colors=colors)
        print(f"  {width}px {colors}c: {size / 1e6:.2f} MB")
        if size <= 3.5e6:
            return path, size


def main(which):
    bd = Board()
    if which in ("still", "both"):
        r = Renderer(bd, 1600, 900)
        print(S.save_still(r.frame(3.78), "switchboard"))
    if which in ("anim", "both"):
        r = Renderer(bd, 960, 540)
        frames = [r.frame(1.0 + i / FPS) for i in range(FPS * 9)]
        print(pack(frames, "switchboard", FPS))
        K.contact_sheet([frames[i] for i in (10, 30, 40, 80)], os.path.join(S.OUT, "switchboard-sheet.png"),
                        width=1600)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "both")
