"""Mockup of isotop's mold view: a Jones (2010) Physarum agent model in a Petri dish where every
process is an oat flake (area ~ memory) that emits attractant in proportion to its CPU. Socket and
pipe links lay weak scent corridors. Renders an isometric dish still and an animation: the mold
grows from the flakes and wires them up, compiler-90 exits and its veins retract, ffmpeg-311
appears and the fringe finds it.

usage: python3 -I mold.py [still|anim|test]
"""
import math
import os
import sys

sys.path.insert(0, "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto")
import isostyle as S  # noqa: E402

import numpy as np  # noqa: E402
from PIL import Image, ImageDraw, ImageFilter  # noqa: E402
from scipy import ndimage  # noqa: E402

# Jones 2010, the most-cited setting.
SA = math.radians(22.5)
RA = math.radians(45.0)
SO = 9.0
SS = 1.0
DEPOSIT = 5.0
DECAY = 0.1
HUNGER = 1500
EMIT0, EMIT = 0.15, 0.012         # steps without food before an agent reappears at a flake

GIB = 1024.0

# name, kind, memory MiB, CPU %, (x, y) in the unit dish (centre 0.5, radius 0.5)
PROCS = [
    ("postgres-41", "system", 1640, 38, (0.30, 0.34)),
    ("worker-134", "container", 900, 95, (0.62, 0.22)),
    ("browser-512", "session", 2450, 64, (0.70, 0.62)),
    ("compiler-90", "session", 1150, 140, (0.40, 0.70)),
    ("language-server-71", "session", 1300, 22, (0.22, 0.58)),
    ("nginx-128", "system", 210, 12, (0.46, 0.13)),
    ("redis-207", "container", 380, 18, (0.82, 0.38)),
    ("journald-310", "system", 120, 2.5, (0.16, 0.38)),
    ("systemd-1", "system", 48, 0.8, (0.50, 0.46)),
    ("pipewire-95", "session", 60, 4, (0.84, 0.70)),
    ("shell-170", "session", 14, 0.4, (0.58, 0.87)),
    ("dbus-114", "system", 18, 1.2, (0.30, 0.17)),
    ("tracker-191", "session", 140, 6, (0.15, 0.73)),
    ("kworker-22", "kernel", 8, 1.5, (0.64, 0.45)),
]
NEWBORN = ("ffmpeg-311", "session", 520, 70, (0.86, 0.53))

LINKS = [
    ("worker-134", "postgres-41"), ("nginx-128", "worker-134"), ("worker-134", "redis-207"),
    ("browser-512", "pipewire-95"), ("compiler-90", "language-server-71"),
    ("journald-310", "systemd-1"),
]


class Flake:
    def __init__(self, n, name, kind, mem, cpu, pos, rng):
        self.name, self.kind, self.mem, self.cpu = name, kind, mem, cpu
        self.cx, self.cy = pos[0] * n, pos[1] * n
        # Area proportional to memory, with a floor so tiny processes are still crumbs.
        area = (70.0 + mem * 0.24) * (n / 400.0) ** 2
        self.r = math.sqrt(area / math.pi)
        self.angle = rng.uniform(0, math.pi)
        self.alive = True
        self.fade = 1.0      # 1 visible, fades to 0 after exit
        self.born = 1.0      # fades in from 0 on birth

    def mask(self, yy, xx, grow=0.0):
        c, s = math.cos(self.angle), math.sin(self.angle)
        dx, dy = xx - self.cx, yy - self.cy
        u = (dx * c + dy * s) / (self.r * 1.25 + grow)
        v = (-dx * s + dy * c) / (self.r * 0.8 + grow)
        return u * u + v * v


class Mold:
    def __init__(self, n=400, agents=70000, start=6000, seed=5, procs=PROCS):
        self.n = n
        self.rng = np.random.default_rng(seed)
        self.R = n * 0.485
        self.c = n / 2.0
        yy, xx = np.mgrid[0:n, 0:n].astype(np.float32)
        self.yy, self.xx = yy, xx
        self.inside = (xx - self.c) ** 2 + (yy - self.c) ** 2 < self.R ** 2
        self.trail = np.zeros((n, n), np.float32)
        self.flakes = [Flake(n, *p, self.rng) for p in procs]
        self.cap = agents
        self.x = np.zeros(0, np.float32)
        self.y = np.zeros(0, np.float32)
        self.h = np.zeros(0, np.float32)
        self.hunger = np.zeros(0, np.int32)
        self.corridor = np.zeros((n, n), np.float32)
        self.density = np.zeros((n, n), np.float32)   # EMA of agents per cell: the plasmodium
        self.tube = np.zeros((n, n), np.float32)      # slow EMA: persistent tubes thicken
        self.build_fields()
        self.spawn(start)
        self.steps = 0

    # Food and corridors -------------------------------------------------------------------
    def build_fields(self):
        n = self.n
        self.food = np.zeros((n, n), np.float32)
        self.near = np.zeros((n, n), bool)
        self.blocked = np.zeros(n * n, bool)
        self.flake_masks = []
        for f in self.flakes:
            d = f.mask(self.yy, self.xx)
            m = d <= 1.0
            self.flake_masks.append(m)
            if f.alive:
                halo = np.exp(-np.maximum(d - 1.0, 0) * 2.5)   # scent reaches a few radii out
                self.food += halo.astype(np.float32) * (EMIT0 + EMIT * f.cpu) * f.born
                self.near |= f.mask(self.yy, self.xx, grow=4.0) <= 1.0
                # The flake itself is solid: the plasmodium wraps its rim.
                self.blocked |= (f.mask(self.yy, self.xx, grow=-1.0) <= 1.0).ravel()
        self.corridor[:] = 0
        names = {f.name: f for f in self.flakes}
        for a, b in LINKS:
            fa, fb = names.get(a), names.get(b)
            if fa is None or fb is None or not (fa.alive and fb.alive):
                continue
            length = int(math.hypot(fb.cx - fa.cx, fb.cy - fa.cy))
            t = np.linspace(0, 1, length * 2)
            xs = (fa.cx + (fb.cx - fa.cx) * t).astype(int)
            ys = (fa.cy + (fb.cy - fa.cy) * t).astype(int)
            self.corridor[ys, xs] = 0.55
        self.corridor = ndimage.gaussian_filter(self.corridor, 1.2)
        self.food *= self.inside

    def spawn(self, count, at=None):
        live = [f for f in self.flakes if f.alive and f.born > 0.5]
        if not live or count <= 0:
            return
        w = np.array([f.cpu + 0.5 for f in live])
        pick = self.rng.choice(len(live), count, p=w / w.sum())
        ang = self.rng.uniform(0, 2 * math.pi, count)
        # Just outside the flake's oval rim, heading outward.
        grow = self.rng.uniform(1.08, 1.5, count)
        a = np.array([live[i].angle for i in pick])
        ra = np.array([live[i].r * 1.25 for i in pick]) * grow + 1.5
        rb = np.array([live[i].r * 0.8 for i in pick]) * grow + 1.5
        lx, ly = ra * np.cos(ang), rb * np.sin(ang)
        x = np.array([live[i].cx for i in pick]) + lx * np.cos(a) - ly * np.sin(a)
        y = np.array([live[i].cy for i in pick]) + lx * np.sin(a) + ly * np.cos(a)
        h = np.arctan2(y - np.array([live[i].cy for i in pick]), x - np.array([live[i].cx for i in pick]))
        h = h + self.rng.normal(0, 0.4, count)
        if at is None:
            self.x = np.concatenate([self.x, x.astype(np.float32)])
            self.y = np.concatenate([self.y, y.astype(np.float32)])
            self.h = np.concatenate([self.h, h.astype(np.float32)])
            self.hunger = np.concatenate([self.hunger, np.zeros(count, np.int32)])
        else:
            self.x[at], self.y[at], self.h[at], self.hunger[at] = x, y, h, 0

    # Jones 2010 step ------------------------------------------------------------------------
    def sense(self, a):
        n = self.n
        sx = np.clip((self.x + SO * np.cos(a)).astype(np.int32), 0, n - 1)
        sy = np.clip((self.y + SO * np.sin(a)).astype(np.int32), 0, n - 1)
        return self.trail[sy, sx]

    def step(self):
        n = self.n
        self.trail += self.food + self.corridor * 0.25
        F = self.sense(self.h)
        L = self.sense(self.h + SA)
        R = self.sense(self.h - SA)
        keep = (F > L) & (F > R)
        rand = ~keep & (F < L) & (F < R)
        right = ~keep & ~rand & (L < R)
        left = ~keep & ~rand & (R < L)
        coin = self.rng.random(self.h.size) < 0.5
        self.h = self.h + np.where(rand, np.where(coin, RA, -RA), 0) - right * RA + left * RA
        self.h = self.h.astype(np.float32)
        nx = self.x + SS * np.cos(self.h)
        ny = self.y + SS * np.sin(self.h)
        ok = (nx - self.c) ** 2 + (ny - self.c) ** 2 < (self.R - 1.5) ** 2
        # Jones's occupancy rule: one agent per cell. A move into an occupied cell fails and the
        # agent picks a new random heading without depositing. Ties go to a random claimant.
        ox = np.clip(self.x.astype(np.int32), 0, n - 1)
        oy = np.clip(self.y.astype(np.int32), 0, n - 1)
        tx = np.clip(nx.astype(np.int32), 0, n - 1)
        ty = np.clip(ny.astype(np.int32), 0, n - 1)
        old = oy * n + ox
        tgt = ty * n + tx
        same = tgt == old
        occ = np.zeros(n * n, bool)
        occ[old] = True
        free = ok & (same | ~occ[tgt]) & ~self.blocked[tgt]
        cand = np.nonzero(free & ~same)[0]
        order = cand[self.rng.permutation(cand.size)]
        _, first = np.unique(tgt[order], return_index=True)
        moved = np.zeros(self.h.size, bool)
        moved[order[first]] = True
        moved |= free & same
        self.x = np.where(moved, nx, self.x).astype(np.float32)
        self.y = np.where(moved, ny, self.y).astype(np.float32)
        bad = ~moved
        self.h[bad] = self.rng.uniform(0, 2 * math.pi, bad.sum())
        ix = np.clip(self.x.astype(np.int32), 0, n - 1)
        iy = np.clip(self.y.astype(np.int32), 0, n - 1)
        flat = iy * n + ix
        self.trail += (np.bincount(flat[moved], minlength=n * n).reshape(n, n) * DEPOSIT).astype(np.float32)
        self.trail = ndimage.uniform_filter(self.trail, 3, mode="constant")
        self.trail *= (1.0 - DECAY)
        self.trail *= self.inside
        occ_now = np.bincount(flat, minlength=n * n).reshape(n, n).astype(np.float32)
        self.density += (occ_now - self.density) * 0.08
        self.tube += (occ_now - self.tube) * 0.006
        # Agents that wander foodless for too long reappear at food, weighted by CPU.
        fed = self.near[iy, ix]
        self.hunger = np.where(fed, 0, self.hunger + 1).astype(np.int32)
        starving = np.nonzero(self.hunger > HUNGER)[0]
        if starving.size:
            self.spawn(starving.size, at=starving)
        self.steps += 1

    def grow(self, count):
        """Biomass grows where the plasmodium already is: new agents bud off random agents."""
        room = min(self.cap - self.x.size, count)
        if room <= 0:
            return
        src = self.rng.integers(0, self.x.size, room)
        self.x = np.concatenate([self.x, self.x[src] + self.rng.normal(0, 0.7, room).astype(np.float32)])
        self.y = np.concatenate([self.y, self.y[src] + self.rng.normal(0, 0.7, room).astype(np.float32)])
        self.h = np.concatenate([self.h, self.rng.uniform(0, 2 * math.pi, room).astype(np.float32)])
        self.hunger = np.concatenate([self.hunger, np.zeros(room, np.int32)])

    # Measurements ---------------------------------------------------------------------------
    def veins(self):
        return ndimage.gaussian_filter(self.trail, 0.8) > 3.0

    def connected(self):
        v = self.veins()
        lab, count = ndimage.label(v)
        live = [(f, m) for f, m in zip(self.flakes, self.flake_masks) if f.alive]
        touch = []
        for f, m in live:
            dm = ndimage.binary_dilation(m, iterations=3)
            ids = set(np.unique(lab[dm])) - {0}
            touch.append(ids)
        tally = {}
        for ids in touch:
            for i in ids:
                tally[i] = tally.get(i, 0) + 1
        if not tally:
            return 0, len(live), v
        main = max(tally, key=tally.get)
        return sum(1 for ids in touch if main in ids), len(live), v


# Rendering ---------------------------------------------------------------------------------

AGAR = np.array([17, 18, 9], np.float32)
_CACHE = {}


def ramp(t, stops, cols):
    out = np.empty(t.shape + (3,), np.float32)
    cols = np.asarray(cols, np.float32)
    for ch in range(3):
        out[..., ch] = np.interp(t, stops, cols[:, ch])
    return out


def vein_colour(t):
    """Physarum yellow on dark agar; t is plasmodium density mapped to 0..1."""
    return ramp(t, [0.0, 0.15, 0.35, 0.62, 0.86, 1.0],
                [[17, 18, 9], [86, 64, 6], [190, 136, 8], [246, 192, 22], [255, 218, 70], [255, 238, 150]])


def flake_layer(mold, size):
    """RGBA texture of the oat flakes at top-down resolution `size`, drawn in bounding boxes."""
    k = size / mold.n
    rgba = np.zeros((size, size, 4), np.float32)
    for f in mold.flakes:
        a = f.fade * f.born
        if a <= 0.01:
            continue
        ext = f.r * 1.4 + 2
        x0, x1 = max(int((f.cx - ext) * k), 0), min(int((f.cx + ext) * k) + 2, size)
        y0, y1 = max(int((f.cy - ext) * k), 0), min(int((f.cy + ext) * k) + 2, size)
        yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
        yy, xx = (yy + 0.5) / k, (xx + 0.5) / k
        d = f.mask(yy, xx)
        inside = np.clip((1.0 - np.sqrt(d)) * f.r * k * 0.8, 0, 1)
        # Oat flake: pale cream, darker rim, rolled fibre streaks, faint kind tint.
        c, s = math.cos(f.angle), math.sin(f.angle)
        along = (xx - f.cx) * c + (yy - f.cy) * s
        across = -(xx - f.cx) * s + (yy - f.cy) * c
        fibre = np.sin(xx * 12.9898 + yy * 78.233) * 43758.5453 % 1.0  # speckle
        base = np.array([222, 208, 168], np.float32)
        tint = np.array(S.KIND[f.kind], np.float32)
        col = base * 0.70 + tint * 0.30
        dome = np.sqrt(np.clip(1.0 - d, 0, 1))
        light = 0.72 + 0.28 * dome - 0.10 * np.clip(along / (f.r * 1.25), -1, 1)
        col = col[None, None, :] * (light * (0.95 + 0.05 * fibre))[..., None]
        alpha = inside * a * 0.95
        reg = rgba[y0:y1, x0:x1]
        reg[..., :3] = reg[..., :3] * (1 - alpha[..., None]) + col * alpha[..., None]
        reg[..., 3] = np.maximum(reg[..., 3], alpha)
    return rgba


def upsample(field, size, order=3):
    im = Image.fromarray(field.astype(np.float32), mode="F").resize((size, size), Image.BICUBIC if order == 3 else Image.BILINEAR)
    return np.maximum(np.asarray(im, np.float32), 0)


def agar_base(size):
    key = ("agar", size)
    if key not in _CACHE:
        rng = np.random.default_rng(42)
        yy, xx = np.mgrid[0:size, 0:size].astype(np.float32) / size
        rr = np.sqrt((xx - 0.5) ** 2 + (yy - 0.5) ** 2) / 0.5
        m1 = ndimage.gaussian_filter(rng.standard_normal((48, 48)).astype(np.float32), 2.5)
        m1 = upsample(m1 + 5, size) - 5
        m2 = upsample(rng.standard_normal((size // 3, size // 3)).astype(np.float32) * 0.5 + 3, size, 1) - 3
        agar = AGAR[None, None, :] * (1.18 - 0.42 * rr[..., None] ** 2)
        agar += (m1 * 9 + m2 * 3)[..., None] * np.array([0.55, 0.6, 0.25], np.float32)
        # A soft sheen where the light catches the moist agar, upper left.
        agar += np.exp(-(((xx - 0.34) / 0.20) ** 2 + ((yy - 0.26) / 0.13) ** 2))[..., None] * np.array([16, 18, 9], np.float32)
        _CACHE[key] = agar
    return _CACHE[key]


def topdown(mold, size, phase=0.0):
    """Top-down colour of the agar, flakes and plasmodium at size x size; returns colour and the
    vein intensity (for bloom)."""
    dens = ndimage.gaussian_filter(mold.density, 0.5)
    body = ndimage.gaussian_filter(mold.tube, 2.0)
    d = upsample(dens, size)
    b = upsample(body, size)
    # Shuttle streaming: a slow peristaltic brightness wave along the veins (decorative).
    if phase:
        yy, xx = np.mgrid[0:size, 0:size].astype(np.float32) * (mold.n / size)
        wave = 1.0 + 0.12 * np.sin(phase - 0.11 * (xx * 0.8 + yy * 0.6) - 0.6 * np.sin(xx * 0.03))
    else:
        wave = 1.0
    # Fine structure sets coverage; the broad flux sets brightness, so thick veins burn pale
    # yellow and the fringe stays a fine ochre lace.
    cover = 1.0 - np.exp(-d * 3.2)
    level = np.clip(b / 0.75, 0, 1) ** 0.8 * wave
    t = np.clip(cover * (0.28 + 0.72 * level), 0, 1)
    agar = agar_base(size)
    flakes = flake_layer(mold, size)
    base = agar * (1 - flakes[..., 3:4]) + flakes[..., :3] * flakes[..., 3:4]
    vein = vein_colour(np.clip(0.22 + 0.48 * level + 0.30 * cover, 0, 1))
    w = (np.clip(cover * 1.25, 0, 1) * (1 - 0.55 * flakes[..., 3]))[..., None]
    col = base * (1 - w) + vein * w
    haze = upsample(ndimage.gaussian_filter(mold.trail - mold.food * 8, 0.6), size)
    haze = np.clip(haze / 30.0, 0, 1) ** 0.7 * (1 - flakes[..., 3])
    col += (haze * (1 - w[..., 0]))[..., None] * np.array([48, 40, 8], np.float32)
    # Plasmodium over a flake goes translucent so the flake shows through, yellowed.
    # Where the plasmodium engulfs a flake, the flake yellows.
    col += flakes[..., 3:4] * cover[..., None] * np.array([30, 22, -10], np.float32)
    return col, t * (1 - 0.6 * flakes[..., 3])


def soft_ellipse(xx, yy, cx, cy, a, b):
    q = np.sqrt(((xx - cx) / a) ** 2 + ((yy - cy) / b) ** 2)
    return np.clip((1.0 - q) * b + 0.5, 0, 1)


class Dish:
    """Screen geometry of the Petri dish for a canvas size."""

    def __init__(self, w, h, band):
        scene = h - band
        self.A = w * 0.405
        self.B = self.A * 0.5
        self.wall = self.B * 0.085       # rim height above the agar
        self.agar = self.B * 0.05        # agar thickness below its surface
        total = 2 * self.B + self.wall + self.agar
        self.cx = w * 0.5
        self.cy = (scene - total) * 0.5 + self.wall + self.B + scene * 0.005

    def to_screen(self, mold, x, y):
        u, v = x / mold.n, y / mold.n
        # The sim grid's dish has radius 0.485 n centred at 0.5 n.
        u = 0.5 + (u - 0.5) / 0.485 * 0.5
        v = 0.5 + (v - 0.5) / 0.485 * 0.5
        return self.cx + (u - 0.5) * 2 * self.A, self.cy + (v - 0.5) * 2 * self.B


def render(mold, w, h, lines, labels=(), phase=0.0, ss=1, hover=None):
    size = max(11, w // 115)
    band = int(size * 1.45) * len(lines) + 10
    dish = Dish(w, h, band)
    A, B, cx, cy = dish.A, dish.B, dish.cx, dish.cy
    canvas = S.sky(w, h)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)

    # Soft shadow below the dish.
    sh = soft_ellipse(xx, yy, cx + A * 0.03, cy + dish.agar + B * 0.16, A * 1.0, B * 0.98)
    sh = ndimage.gaussian_filter(sh, w / 90)
    canvas *= (1 - 0.6 * sh)[..., None]

    # Agar slab seen through the front glass: a swept ellipse below the surface.
    side = np.zeros((h, w), np.float32)
    steps = max(int(dish.agar) + 1, 2)
    for i in range(steps + 1):
        side = np.maximum(side, soft_ellipse(xx, yy, cx, cy + dish.agar * i / steps + 2, A + 1, B + 1))
    depth = np.clip((yy - (cy + np.sqrt(np.clip(1 - ((xx - cx) / A) ** 2, 0, 1)) * B)) / max(dish.agar, 1), 0, 1)
    slab = np.array([38, 40, 16], np.float32) * (1 - 0.55 * depth[..., None]) + np.array([6, 6, 2], np.float32)
    canvas = canvas * (1 - side[..., None]) + slab * side[..., None]

    # Agar surface: the top-down picture squashed into the dish ellipse.
    D = int(2 * A * 0.97) * ss
    top, t = topdown(mold, D, phase)
    # Circular cut of the sim dish.
    q = np.mgrid[0:D, 0:D].astype(np.float32)
    rr = np.sqrt(((q[0] + 0.5) / D - 0.5) ** 2 + ((q[1] + 0.5) / D - 0.5) ** 2)
    sim_r = 0.485
    # Map the sim dish (radius 0.485 n) to fill the ellipse.
    crop = int(D * (0.5 - sim_r))
    top = top[crop:D - crop, crop:D - crop]
    t = t[crop:D - crop, crop:D - crop]
    tw, th = int(round(2 * A)), int(round(2 * B))
    top_im = S.to_image(top).resize((tw, th), Image.LANCZOS)
    t_im = Image.fromarray((np.clip(t, 0, 1) * 255).astype(np.uint8)).resize((tw, th), Image.LANCZOS)
    x0, y0 = int(round(cx - A)), int(round(cy - B))
    surf = np.zeros((h, w, 3), np.float32)
    tint = np.zeros((h, w), np.float32)
    surf[y0:y0 + th, x0:x0 + tw] = np.asarray(top_im, np.float32)[:h - y0, :w - x0]
    tint[y0:y0 + th, x0:x0 + tw] = np.asarray(t_im, np.float32)[:h - y0, :w - x0] / 255.0
    cover = soft_ellipse(xx, yy, cx, cy, A, B)
    canvas = canvas * (1 - cover[..., None]) + surf * cover[..., None]

    # Bloom of the plasmodium, in screen space so it stays round.
    glow_src = (np.clip(tint - 0.25, 0, 1) ** 1.3) * cover
    g1 = ndimage.gaussian_filter(glow_src, w / 400)
    g2 = ndimage.gaussian_filter(glow_src, w / 110)
    canvas += g1[..., None] * np.array([120, 92, 14], np.float32) * 0.9
    canvas += g2[..., None] * np.array([150, 110, 10], np.float32) * 0.9

    # Glass: back inner wall above the agar, a thin tint against the sky.
    rim_y = cy - dish.wall
    back = np.zeros((h, w), np.float32)
    for i in range(int(dish.wall) + 2):
        back = np.maximum(back, soft_ellipse(xx, yy, cx, cy - i, A, B))
    back = back * (1 - cover)
    canvas += back[..., None] * np.array([22, 26, 34], np.float32)

    img = S.to_image(canvas)
    # Glass outlines drawn at 2x and downsampled for clean anti-aliasing.
    k = 2
    over = Image.new("RGBA", (w * k, h * k), (0, 0, 0, 0))
    dr = ImageDraw.Draw(over)
    lw = max(1, int(w / 800)) * k
    def box(yc, a=A, b=B):
        return [(cx - a) * k, (yc - b) * k, (cx + a) * k, (yc + b) * k]
    bottom_y = cy + dish.agar + 2
    # Rim: two concentric ellipses (the glass thickness), the near arc brighter.
    dr.ellipse(box(rim_y), outline=(200, 214, 230, 120), width=lw)
    dr.ellipse(box(rim_y, A - 5, B - 2.5), outline=(200, 214, 230, 70), width=lw)
    dr.arc(box(rim_y), 0, 180, fill=(220, 232, 245, 170), width=lw)
    dr.arc(box(bottom_y), 0, 180, fill=(190, 205, 225, 110), width=lw)
    # Silhouette edges of the cylinder.
    dr.line([(cx - A) * k, rim_y * k, (cx - A) * k, bottom_y * k], fill=(200, 214, 230, 120), width=lw)
    dr.line([(cx + A) * k, rim_y * k, (cx + A) * k, bottom_y * k], fill=(200, 214, 230, 90), width=lw)
    # Specular highlight on the rim, upper left, and a soft reflection on the near wall.
    dr.arc(box(rim_y), 196, 238, fill=(255, 255, 255, 200), width=lw * 2)
    dr.arc(box(rim_y - 1), 300, 322, fill=(255, 255, 255, 120), width=lw)
    over = over.resize((w, h), Image.LANCZOS)
    img = Image.alpha_composite(img.convert("RGBA"), over)
    # Near-wall reflection streaks (vertical, translucent).
    refl = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    rd = ImageDraw.Draw(refl)
    for fx, alpha, width in ((-0.78, 40, 0.018), (-0.70, 22, 0.008), (0.62, 18, 0.012)):
        x = cx + fx * A
        yb = cy + B * math.sqrt(max(0.0, 1 - fx * fx))
        rd.rectangle([x, yb - dish.wall, x + width * A, yb + dish.agar], fill=(220, 235, 255, alpha))
    refl = refl.filter(ImageFilter.GaussianBlur(w / 900))
    img = Image.alpha_composite(img, refl).convert("RGB")

    draw = ImageDraw.Draw(img)
    fs = max(11, int(w / 112))
    rects = []
    for f, text in labels:
        sx, sy = dish.to_screen(mold, f.cx, f.cy)
        ry = f.r / mold.n / 0.485 * B * 1.25
        a = min(1.0, f.fade * f.born)
        fill = tuple(int(c * a + 20 * (1 - a)) for c in S.TEXT)
        label(draw, sx, sy - ry - fs * 0.9, text, fs, fill=fill)
        tw = draw.textlength(text, font=S.font(fs))
        rects.append((sx - tw / 2 - 4, sy - ry - fs * 1.6, sx + tw / 2 + 4, sy - ry - fs * 0.2))
    if hover:
        (a, b), info = hover
        fa = next(f for f in mold.flakes if f.name == a)
        fb = next(f for f in mold.flakes if f.name == b)
        # Pointer on the densest vein cell along the corridor's middle stretch.
        best, bx, by = -1.0, fa.cx, fa.cy
        for s in np.linspace(0.3, 0.7, 200):
            x = fa.cx + (fb.cx - fa.cx) * s
            y = fa.cy + (fb.cy - fa.cy) * s
            for o in np.linspace(-2, 2, 5):
                nx = -(fb.cy - fa.cy)
                ny = fb.cx - fa.cx
                ln = math.hypot(nx, ny)
                qx, qy = x + nx / ln * o, y + ny / ln * o
                v = mold.density[int(qy), int(qx)] - abs(s - 0.5) * 1.2
                if v > best:
                    best, bx, by = v, qx, qy
        px, py = dish.to_screen(mold, bx, by)
        tooltip(img, px, py, [f"vein on link {a} <-> {b}"] + info, max(11, int(w / 125)), rects)
    S.status(img, lines)
    return img


def label(draw, x, y, text, size, fill=S.TEXT):
    """isotop label with a dark stroke so it reads over bright veins."""
    f = S.font(size)
    draw.text((x + 1, y + 1), text, font=f, fill=(0, 0, 0), anchor="mm", stroke_width=2, stroke_fill=(0, 0, 0))
    draw.text((x, y), text, font=f, fill=fill, anchor="mm", stroke_width=1, stroke_fill=(8, 8, 4))


def tooltip(img, x, y, lines, size, avoid=()):
    """Hover inspector: a small dark panel by the pointer, placed clear of labels."""
    draw = ImageDraw.Draw(img, "RGBA")
    f = S.font(size)
    wmax = max(draw.textlength(t, font=f) for t in lines)
    lh = int(size * 1.4)
    bw, bh = wmax + 16, lh * len(lines) + 10
    for bx, by in ((x + 14, y + 14), (x - bw - 8, y + 14), (x + 14, y - bh - 8), (x - bw - 8, y - bh - 8)):
        if not any(bx < r[2] and bx + bw > r[0] and by < r[3] and by + bh > r[1] for r in avoid):
            break
    draw.rectangle([bx, by, bx + bw, by + bh], fill=(14, 14, 22, 225),
                   outline=(120, 126, 150, 200))
    for i, t in enumerate(lines):
        draw.text((bx + 8, by + 5 + i * lh), t, font=f, fill=S.TEXT if i == 0 else S.DIM)
    # Pointer arrow.
    s = size * 1.1
    pts = [(x, y), (x, y + s), (x + s * 0.28, y + s * 0.74), (x + s * 0.5, y + s * 1.12),
           (x + s * 0.64, y + s * 1.04), (x + s * 0.44, y + s * 0.68), (x + s * 0.8, y + s * 0.68)]
    draw.polygon(pts, fill=(240, 240, 240), outline=(0, 0, 0))


def label_text(f):
    mem = f"{f.mem / GIB:.1f} GiB" if f.mem >= 1000 else f"{f.mem:.0f} MiB"
    return f"{f.name} {f.cpu:.0f}% {mem}"


LEGEND = ("oat flake = process | area = memory | attractant = CPU | scent corridors = socket and pipe traffic"
          " | tint = kind | agents: Jones 2010, SA 22.5 RA 45 SO 9")
KEYS = "Tab next view | g tour | scroll pan | click inspect | / search | c links | Space pause | ? help | q quit"
LABELLED = ["postgres-41", "worker-134", "browser-512", "compiler-90", "language-server-71", "redis-207",
            "nginx-128", "ffmpeg-311"]


def status_lines(mold, t, extra="", short=False):
    c, tot, v = mold.connected()
    if short:
        line = (f"ISOTOP / MOLD / DEMO   192 processes | biomass {mold.x.size / 1000:.1f}k"
                f" | veins {int(v.sum()):,} cells | flakes connected {c}/{tot}")
    else:
        line = (f"ISOTOP / MOLD / DEMO   192 processes | biomass {mold.x.size / 1000:.1f}k agents"
                f" | vein length {int(v.sum()):,} cells | flakes connected {c}/{tot} | t={t:.1f}s")
    if extra:
        line += "   " + extra
    legend = LEGEND if not short else ("flake = process | area = memory | attractant = CPU"
                                       " | corridors = socket traffic | Jones 2010 SA 22.5 RA 45 SO 9")
    return [line, legend, KEYS]


def labels_for(mold, names=LABELLED):
    return [(f, label_text(f)) for f in mold.flakes if f.name in names and f.fade * f.born > 0.3]


def corridor_density(mold, a, b):
    fa = next(f for f in mold.flakes if f.name == a)
    fb = next(f for f in mold.flakes if f.name == b)
    s = np.linspace(0.3, 0.7, 120)
    xs = (fa.cx + (fb.cx - fa.cx) * s).astype(int)
    ys = (fa.cy + (fb.cy - fa.cy) * s).astype(int)
    d = ndimage.maximum_filter(mold.density, 5)
    return float(np.mean(d[ys, xs] > 0.25))


def set_alive(mold, name, alive):
    for f in mold.flakes:
        if f.name == name:
            f.alive = alive
    mold.build_fields()


def still():
    n, agents = 720, 110000
    mold = Mold(n=n, agents=agents, start=agents // 8, procs=PROCS)
    for i in range(1500):
        if i % 10 == 0:
            mold.grow(agents // 40)
        mold.step()
    info = {("worker-134", "postgres-41"): ["loopback tcp 1.4 MB/s, 6 sockets", "scent 0.21 of corridor max"],
            ("nginx-128", "worker-134"): ["loopback tcp 820 kB/s, 14 sockets", "scent 0.17 of corridor max"],
            ("worker-134", "redis-207"): ["loopback tcp 2.1 MB/s, 9 sockets", "scent 0.26 of corridor max"],
            ("browser-512", "pipewire-95"): ["unix stream queue 64 KiB, 3 sockets", "scent 0.09 of corridor max"],
            ("compiler-90", "language-server-71"): ["pipe 310 kB/s, 2 fds", "scent 0.06 of corridor max"],
            ("journald-310", "systemd-1"): ["unix dgram 4 KiB, 41 sockets", "scent 0.04 of corridor max"]}
    best = max(info, key=lambda k: corridor_density(mold, *k))
    print("hover", best, corridor_density(mold, *best))
    hover = (best, info[best])
    img = render(mold, 1600, 900, status_lines(mold, 1500 / 30.0), labels_for(mold), phase=0.0, hover=hover)
    print(S.save_still(img, "mold"))


def anim():
    global HUNGER
    HUNGER = 450
    n = 440
    agents = int(110000 * (n / 720) ** 2)
    procs = PROCS + [NEWBORN]
    mold = Mold(n=n, agents=agents, start=agents // 10, procs=procs)
    newborn = mold.flakes[-1]
    newborn.alive, newborn.born = False, 0.0
    mold.build_fields()
    compiler = next(f for f in mold.flakes if f.name == "compiler-90")
    frames = []
    total, per = 120, 16
    exit_at, birth_at = 52, 80
    wired_at = None
    for fi in range(total):
        extra = ""
        if fi == exit_at:
            compiler.alive = False
            mold.build_fields()
        if fi >= exit_at:
            compiler.fade = max(0.0, 1.0 - (fi - exit_at) / 8.0)
            extra = "compiler-90 exited, veins retract"
        if fi == birth_at:
            newborn.alive = True
            newborn.born = 0.05
        if fi >= birth_at:
            newborn.born = min(1.0, 0.05 + (fi - birth_at) / 6.0)
            mold.build_fields()
            extra = "ffmpeg-311 started, fringe searching"
        for s in range(per):
            if mold.steps % 6 == 0:
                mold.grow(agents // 60)
            mold.step()
        c, tot, v = mold.connected()
        if fi >= birth_at:
            if wired_at is None and c == tot:
                wired_at = fi
            if wired_at is not None:
                extra = "ffmpeg-311 found, wired in"
        lines = status_lines(mold, mold.steps / 30.0, extra, short=True)
        img = render(mold, 960, 540, lines, labels_for(mold, ["postgres-41", "worker-134", "browser-512",
                                                               "compiler-90", "ffmpeg-311", "redis-207"]),
                     phase=fi * 0.45)
        frames.append(img)
        if fi % 20 == 0:
            print("frame", fi, "connected", c, tot, flush=True)
    path, size = S.save_frames(frames, "mold", fps=12)
    import stabgif
    print(stabgif.pack("mold", fps=12, colors=128, thresh=10))


def quick(mold, path):
    col, _ = topdown(mold, mold.n)
    col[~mold.inside] = 0
    S.to_image(col).save(path)


def test():
    mold = Mold(n=400, agents=int(sys.argv[2]) if len(sys.argv) > 2 else 20000, start=3000)
    import time
    t0 = time.time()
    for i in range(900):
        if i % 10 == 0:
            mold.grow(600)
        mold.step()
    c, tot, v = mold.connected()
    print(f"{time.time() - t0:.1f}s connected {c}/{tot} veins {v.sum()}", flush=True)
    names = {"postgres-41", "worker-134", "browser-512", "compiler-90", "nginx-128", "redis-207"}
    labels = [(f, label_text(f)) for f in mold.flakes if f.name in names]
    img = render(mold, 1600, 900, ["ISOTOP / MOLD / DEMO   test", "legend"], labels, phase=0.0)
    img.save(f"{S.OUT}/mold-test-iso.png")
    print(f"{time.time() - t0:.1f}s rendered", flush=True)


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "test"
    if mode == "test":
        test()
    elif mode == "evolve":
        evolve()
    elif mode == "still":
        still()
    elif mode == "anim":
        anim()
