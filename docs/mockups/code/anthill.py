"""Mockup of isotop's anthill view: syscalls as foraging ants.

Every process is an ant nest (mound size ~ memory). Every open regular file is a food pile on a
treemap of the filesystem. One ant walks per K syscalls: read ants go nest -> file and come home
with a crumb, write ants carry a crumb out. Crumb size is bytes per syscall (rchar/syscr,
wchar/syscw). Routes are emergent: a small ant-colony model (steer to target + follow pheromone
+ noise, pheromone evaporates). Pipes and sockets are dashed tunnels between nests.

usage: python3 -I anthill.py [still|anim|both]
"""
import math
import os
import sys

PROTO = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, PROTO)

import cv2  # noqa: E402
import numpy as np  # noqa: E402
from PIL import ImageDraw  # noqa: E402

import isostyle as S  # noqa: E402
import mockkit as K  # noqa: E402

KANT = 50                 # syscalls per ant
SPEED = 0.20              # world units per second
TAU = 2.2                 # pheromone evaporation time constant, seconds
G = 384                   # pheromone grid cells per world unit

READ_CRUMB = np.array([255, 238, 196], np.float32)
WRITE_CRUMB = np.array([255, 118, 92], np.float32)
ANT = np.array([22, 15, 11], np.float32)

# Treemap of the directory tree: a pinwheel around the colony clearing.
CLEAR = (0.24, 0.26, 0.74, 0.79)  # x0, y0, x1, y1
REGIONS = [
    ("/usr", (0.0, 0.0, 0.74, 0.26), (4, 6, 12)),
    ("/var/log", (0.74, 0.0, 1.0, 0.38), (6, 4, 2)),
    ("/var/lib", (0.74, 0.38, 1.0, 0.79), (2, 5, 8)),
    ("/tmp", (0.24, 0.79, 1.0, 1.0), (0, 7, 5)),
    ("/home/tom", (0.0, 0.26, 0.24, 1.0), (10, 6, 0)),
]

# name: kind, x, y, memory MiB, bytes per read, bytes per write
NESTS = {
    "systemd-1": ("system", 0.330, 0.330, 14, 4096, 512),
    "journald-310": ("system", 0.480, 0.320, 62, 1024, 512),
    "nginx-128": ("system", 0.650, 0.340, 44, 32768, 190),
    "browser-512": ("session", 0.375, 0.495, 2150, 16384, 8192),
    "postgres-41": ("container", 0.590, 0.470, 1230, 8192, 8192),
    "language-server-71": ("session", 0.285, 0.700, 910, 4096, 512),
    "compiler-90": ("session", 0.470, 0.640, 640, 131072, 4096),
    "redis-207": ("container", 0.680, 0.590, 310, 512, 65536),
    "worker-134": ("container", 0.625, 0.720, 150, 64, 256),
    "shell-170": ("session", 0.465, 0.765, 8, 1, 24),
}

# key: path, label, x, y, size bytes
FILES = {
    "libc": ("/usr/lib/x86_64-linux-gnu/libc.so.6", "libc.so.6", 0.17, 0.12, 2.2e6),
    "llvm": ("/usr/lib/libLLVM.so.18.1", "libLLVM.so.18.1", 0.42, 0.10, 1.2e8),
    "html": ("/usr/share/nginx/html/index.html", "index.html", 0.64, 0.14, 9.0e5),
    "journal": ("/var/log/journal/system.journal", "system.journal", 0.86, 0.10, 1.3e8),
    "access": ("/var/log/nginx/access.log", "nginx/access.log", 0.88, 0.28, 4.8e7),
    "pgbase": ("/var/lib/postgresql/16/main/base/16384", "postgres/base/16384", 0.88, 0.49, 1.07e9),
    "rdb": ("/var/lib/redis/dump.rdb", "redis/dump.rdb", 0.88, 0.69, 3.1e8),
    "mainrs": ("/home/tom/src/isotop/src/main.rs", "src/main.rs", 0.12, 0.40, 6.2e4),
    "cache": ("/home/tom/.cache/browser/data_1", "cache/data_1", 0.055, 0.69, 3.6e7),
    "buildlog": ("/home/tom/src/isotop/build.log", "build.log", 0.13, 0.84, 3.4e5),
    "hist": ("/home/tom/.bash_history", ".bash_history", 0.065, 0.955, 1.8e4),
    "ccs": ("/tmp/cc1-4f2a.s", "cc1-4f2a.s", 0.47, 0.905, 4.1e6),
    "cco": ("/tmp/cc1-8e1d.o", "cc1-8e1d.o", 0.78, 0.905, 2.2e6),
    "tty": ("/dev/pts/3", "/dev/pts/3", 0.345, 0.790, 0),
}

T_OPEN = 1.5     # compiler-90 opens /tmp/cc1-8e1d.o
T_CLOSE = 5.0    # browser-512 closes cache/data_1

# nest, file, op, syscalls per second
FLOWS = [
    ("postgres-41", "pgbase", "r", 7200),
    ("worker-134", "access", "r", 9600),
    ("compiler-90", "llvm", "r", 1900),
    ("compiler-90", "libc", "r", 500),
    ("compiler-90", "mainrs", "r", 600),
    ("language-server-71", "mainrs", "r", 2400),
    ("browser-512", "cache", "r", 3100),
    ("nginx-128", "html", "r", 1500),
    ("shell-170", "tty", "r", 60),
    ("journald-310", "journal", "w", 1800),
    ("nginx-128", "access", "w", 1500),
    ("postgres-41", "pgbase", "w", 900),
    ("compiler-90", "buildlog", "w", 300),
    ("compiler-90", "ccs", "w", 1000),
    ("compiler-90", "cco", "w", 1300),
    ("redis-207", "rdb", "w", 600),
    ("browser-512", "cache", "w", 700),
    ("shell-170", "hist", "w", 40),
]

# Pipes and sockets: from, to, what
TUNNELS = [
    ("shell-170", "compiler-90", "pipe"),
    ("nginx-128", "worker-134", "unix"),
    ("worker-134", "postgres-41", "tcp lo"),
    ("systemd-1", "journald-310", "unix"),
    ("worker-134", "redis-207", "unix"),
]


def nest_radius(mem):
    return 0.014 + 0.0060 * math.log2(max(mem, 8) / 8)


def pile_radius(size):
    if size <= 0:
        return 0.035
    return 0.007 + 0.0019 * max(0.0, math.log2(size / 4096))


def flow_bend(fi):
    """Stable per-flow curvature so streams fan out of a nest instead of overlapping."""
    rng = np.random.default_rng(1000 + fi * 7919)
    return float(rng.choice([-1, 1]) * rng.uniform(0.10, 0.24))


def crumb_px(nbytes):
    """Crumb radius in output pixels at the 1600 px still: log of bytes per syscall."""
    return float(np.clip(0.55 + 0.27 * math.log2(max(nbytes, 1) / 64), 0.42, 3.6))


def flow_rate(f, t):
    nest, key, op, calls = f
    r = calls / KANT
    if key == "cco":
        if t < T_OPEN:
            return 0.0
        return r * min(1.0, (t - T_OPEN) / 2.5)
    if key == "cache" and t >= T_CLOSE:
        return 0.0
    return r


def file_alpha(key, t):
    if key == "cco":
        return float(np.clip((t - T_OPEN) / 0.6, 0, 1))
    if key == "cache":
        return float(np.clip(1 - (t - T_CLOSE - 1.5) / 1.5, 0, 1))
    return 1.0


def wrap(a):
    return (a + np.pi) % (2 * np.pi) - np.pi


class Colony:
    def __init__(self, seed=11, nmax=9000):
        self.rng = np.random.default_rng(seed)
        self.P = np.zeros((3, G, G), np.float32)
        self.n = nmax
        self.alive = np.zeros(nmax, bool)
        self.pos = np.zeros((nmax, 2))
        self.ang = np.zeros(nmax)
        self.state = np.zeros(nmax, np.int8)
        self.flow = np.zeros(nmax, np.int32)
        self.speed = np.zeros(nmax)
        self.tgt = np.zeros((nmax, 2))
        self.wander = np.zeros(nmax)
        self.acc = np.zeros(len(FLOWS))
        self.t = 0.0
        self.npos = np.array([[NESTS[f[0]][1], NESTS[f[0]][2]] for f in FLOWS])
        self.nrad = np.array([nest_radius(NESTS[f[0]][3]) for f in FLOWS])
        self.fpos = np.array([[FILES[f[1]][2], FILES[f[1]][3]] for f in FLOWS])
        self.frad = np.array([pile_radius(FILES[f[1]][4]) for f in FLOWS])
        self.isw = np.array([f[2] == "w" for f in FLOWS])
        self.col = np.array([S.KIND[NESTS[f[0]][0]] for f in FLOWS], np.float32) / 255.0
        d = self.fpos - self.npos
        perp = np.c_[-d[:, 1], d[:, 0]]
        self.ctrl = (self.npos + self.fpos) / 2 + perp * np.array([flow_bend(i) for i in range(len(FLOWS))])[:, None]
        self.span = np.hypot(d[:, 0], d[:, 1])

    def spawn(self, fi, k, t):
        free = np.flatnonzero(~self.alive)[:k]
        k = len(free)
        if k == 0:
            return
        rng = self.rng
        nx, ny = self.npos[fi]
        fx, fy = self.fpos[fi]
        base = math.atan2(fy - ny, fx - nx)
        th = base + rng.normal(0, 0.55, k)
        rr = self.nrad[fi] * rng.uniform(0.55, 0.95, k)
        self.pos[free] = np.c_[nx + rr * np.cos(th), ny + rr * np.sin(th)]
        self.ang[free] = th + rng.normal(0, 0.3, k)
        back = base + math.pi + rng.normal(0, 0.8, k)
        fr = self.frad[fi] * rng.uniform(0.6, 1.0, k)
        self.tgt[free] = np.c_[fx + fr * np.cos(back), fy + fr * np.sin(back)]
        self.state[free] = 0
        self.flow[free] = fi
        self.speed[free] = SPEED * np.clip(rng.normal(1, 0.13, k), 0.7, 1.3)
        # Scouts on a freshly opened file wander more until the trail is laid.
        scout = 0.0
        if FLOWS[fi][1] == "cco":
            scout = max(0.0, 1.0 - (t - T_OPEN) / 3.5)
        self.wander[free] = 1.0 + 3.0 * scout
        self.alive[free] = True

    def sense(self, field, x, y):
        ix = np.clip((x * G).astype(np.int32), 0, G - 1)
        iy = np.clip((y * G).astype(np.int32), 0, G - 1)
        return field[iy, ix]

    def step(self, dt):
        t = self.t
        rng = self.rng
        for fi, f in enumerate(FLOWS):
            self.acc[fi] += flow_rate(f, t) * dt
            k = int(self.acc[fi])
            if k:
                self.acc[fi] -= k
                self.spawn(fi, k, t)
        idx = np.flatnonzero(self.alive)
        if len(idx):
            pos, ang = self.pos[idx], self.ang[idx]
            fi = self.flow[idx]
            st = self.state[idx]
            to = self.tgt[idx] - pos
            d = np.hypot(to[:, 0], to[:, 1])
            want = np.arctan2(to[:, 1], to[:, 0])
            # A carrot ahead on the flow's curved route; outbound runs nest->file, home file->nest.
            u = np.clip(1 - d / (self.span[fi] + 1e-6), 0, 1)
            u = np.minimum(u + 0.14, 1.0)
            uu = np.where(st == 0, u, 1 - u)[:, None]
            c = (1 - uu) ** 2 * self.npos[fi] + 2 * (1 - uu) * uu * self.ctrl[fi] + uu ** 2 * self.fpos[fi]
            carrot = np.where((d < 0.05)[:, None], self.tgt[idx], c)
            want = np.arctan2(carrot[:, 1] - pos[:, 1], carrot[:, 0] - pos[:, 0])
            field = self.P.sum(0)
            sa, sd = 0.5, 0.02
            sl = self.sense(field, pos[:, 0] + sd * np.cos(ang - sa), pos[:, 1] + sd * np.sin(ang - sa))
            sr = self.sense(field, pos[:, 0] + sd * np.cos(ang + sa), pos[:, 1] + sd * np.sin(ang + sa))
            steer = (sr - sl) / (sr + sl + 0.08)
            wnd = self.wander[idx]
            kt = 3.2 / wnd
            ang = ang + dt * (kt * wrap(want - ang) + 3.0 * steer) \
                + np.sqrt(dt) * 0.55 * wnd * rng.standard_normal(len(idx))
            near = d < 0.035
            ang = np.where(near, want, ang)
            sp = self.speed[idx]
            pos = pos + (sp * dt)[:, None] * np.c_[np.cos(ang), np.sin(ang)]
            pos = np.clip(pos, 0.004, 0.996)
            self.pos[idx], self.ang[idx] = pos, ang
            # Arrivals: at the pile turn round with (read) or without (write) a crumb; at home, done.
            arrived = d < max(0.012, SPEED * dt * 1.2)
            out = idx[arrived & (st == 0)]
            if len(out):
                fi = self.flow[out]
                base = np.arctan2(self.npos[fi, 1] - self.fpos[fi, 1], self.npos[fi, 0] - self.fpos[fi, 0])
                th = base + math.pi + rng.normal(0, 0.6, len(out))
                rr = self.nrad[fi] * 0.5
                self.tgt[out] = self.npos[fi] - np.c_[rr * np.cos(th), rr * np.sin(th)]
                self.ang[out] += math.pi + rng.normal(0, 0.4, len(out))
                self.state[out] = 1
                self.wander[out] = np.minimum(self.wander[out], 1.4)
            home = idx[arrived & (st == 1)]
            self.alive[home] = False
            # Pheromone: laid more heavily by ants carrying food home, as real foragers do.
            idx = np.flatnonzero(self.alive)
            carrying = np.where(self.isw[self.flow[idx]], self.state[idx] == 0, self.state[idx] == 1)
            w = np.where(carrying, 1.0, 0.45) * dt
            ix = np.clip((self.pos[idx, 0] * G).astype(np.int64), 0, G - 1)
            iy = np.clip((self.pos[idx, 1] * G).astype(np.int64), 0, G - 1)
            flat = iy * G + ix
            col = self.col[self.flow[idx]]
            for c in range(3):
                self.P[c].ravel()[:] += np.bincount(flat, weights=w * col[:, c], minlength=G * G).astype(np.float32)
        self.P *= math.exp(-dt / TAU)
        for c in range(3):
            self.P[c] = cv2.GaussianBlur(self.P[c], (0, 0), 0.42)
        self.t += dt

    def run(self, seconds, dt=1 / 24):
        for _ in range(int(round(seconds / dt))):
            self.step(dt)

    def ants(self):
        idx = np.flatnonzero(self.alive)
        fi = self.flow[idx]
        carrying = np.where(self.isw[fi], self.state[idx] == 0, self.state[idx] == 1)
        return self.pos[idx], self.ang[idx], fi, carrying


# ---------------------------------------------------------------------------------------------
# Static textures


def sand_texture(n, seed=3):
    rng = np.random.default_rng(seed)
    u = (np.arange(n, dtype=np.float32) + 0.5) / n
    X, Y = np.meshgrid(u, u)
    warp = K.value_noise(n, 5, rng, ((1, 1.0), (2.3, 0.5)))
    phase = (X * 0.83 + Y * 0.56) * 95 + warp * 2.4
    ripple = np.sin(phase) + 0.38 * np.sin(2 * phase + 0.9)
    amp = np.clip(0.55 + 0.5 * K.value_noise(n, 4, rng), 0.05, 1.0)
    dunes = K.value_noise(n, 4, rng, ((1, 1.0), (2.1, 0.45), (4.7, 0.2)))
    h = 0.0024 * ripple * amp + 0.010 * dunes
    gy, gx = np.gradient(h, 1.0 / n)
    nrm = np.dstack([-gx, -gy, np.ones_like(h)])
    nrm /= np.linalg.norm(nrm, axis=2, keepdims=True)
    diffuse = np.clip(nrm @ K.LIGHT, 0, 1)
    base = np.array([100, 83, 65], np.float32)
    tone = 1.0 + 0.10 * K.value_noise(n, 9, rng)
    grain = 1.0 + 0.10 * rng.standard_normal((n, n)).astype(np.float32)
    tex = base * (0.30 + 0.72 * diffuse[..., None]) * (tone * grain)[..., None]
    # Pebbles: sparse darker and lighter specks.
    specks = rng.random((n, n)) < 0.0035
    tex[specks] *= rng.uniform(0.55, 1.35, specks.sum())[:, None].astype(np.float32)
    # Regions: a faint tint and a fine string outline, like a surveyed dig.
    for name, (x0, y0, x1, y1), tint in REGIONS:
        m = 0.007
        a0, b0, a1, b1 = [int(v * n) for v in (x0 + m, y0 + m, x1 - m, y1 - m)]
        tex[b0:b1, a0:a1] += np.array(tint, np.float32)
        lw = max(2, n // 700)
        edge = np.zeros((n, n), np.uint8)
        cv2.rectangle(edge, (a0, b0), (a1, b1), 255, lw, cv2.LINE_AA)
        e = edge.astype(np.float32)[..., None] / 255.0
        tex = tex * (1 - 0.55 * e) + np.array([196, 178, 150], np.float32) * 0.55 * e
    # Darken toward the rim so the slab reads as a lit patch at night.
    rim = np.minimum(np.minimum(X, 1 - X), np.minimum(Y, 1 - Y))
    tex *= (0.72 + 0.28 * np.clip(rim / 0.12, 0, 1))[..., None]
    tex *= (0.88 + 0.12 * np.clip(1 - np.hypot(X - 0.5, Y - 0.55) / 0.6, 0, 1))[..., None]
    return tex.astype(np.float32)


def blanket(tex, n):
    """Gingham picnic blanket for the terminal, painted flat into the ground texture."""
    _, _, bx, by, _ = FILES["tty"]
    side, rot = 0.085, math.radians(18)
    u = (np.arange(n, dtype=np.float32) + 0.5) / n
    X, Y = np.meshgrid(u, u)
    dx, dy = X - bx, Y - by
    a = (dx * math.cos(rot) + dy * math.sin(rot)) / side
    b = (-dx * math.sin(rot) + dy * math.cos(rot)) / side
    sh_a = ((dx - 0.006) * math.cos(rot) + (dy - 0.004) * math.sin(rot)) / side
    sh_b = (-(dx - 0.006) * math.sin(rot) + (dy - 0.004) * math.cos(rot)) / side
    shadow = (np.abs(sh_a) < 0.5) & (np.abs(sh_b) < 0.5)
    tex[shadow] *= 0.6
    inside = (np.abs(a) < 0.5) & (np.abs(b) < 0.5)
    checks = 9
    sa = (np.floor((a + 0.5) * checks) % 2).astype(np.float32)
    sb = (np.floor((b + 0.5) * checks) % 2).astype(np.float32)
    red = np.array([168, 36, 40], np.float32)
    cream = np.array([214, 200, 176], np.float32)
    mix = (sa + sb) / 2
    cloth = cream * (1 - mix[..., None]) + red * mix[..., None]
    wr = 0.86 + 0.1 * np.sin(a * 11 + b * 3) + 0.05 * np.sin(b * 17 - a * 2)
    edge = np.clip((0.5 - np.maximum(np.abs(a), np.abs(b))) * 40, 0, 1)
    cloth = cloth * (wr * (0.72 + 0.28 * edge))[..., None] * 0.74
    tex[inside] = cloth[inside]


# ---------------------------------------------------------------------------------------------
# Scene objects


def render_mound(rgb, alpha, glows, geo, x, y, R, H, kind_col, rng):
    sx, sy = geo.p(x, y)
    s = geo.s
    a = math.sqrt(2) * R * s
    b = a / 2
    top = H * s
    hgt, wid = alpha.shape
    x0, x1 = max(int(sx - a - 3), 0), min(int(sx + a + 4), wid)
    y0, y1 = max(int(sy - top - b - 3), 0), min(int(sy + b + 4), hgt)
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    du, dv = xx - sx, yy - sy
    p = 1.6
    zk = np.full(du.shape, -1.0, np.float32)
    rk = np.zeros(du.shape, np.float32)
    cov = np.zeros(du.shape, np.float32)
    steps = 72
    rtop = 0.22 * R
    for k in range(steps):
        z = H * k / (steps - 1)
        r = max(R * max(0.0, 1 - z / H) ** (1 / p), rtop)
        ak = math.sqrt(2) * r * s
        bk = ak / 2
        q = (du / ak) ** 2 + ((dv + z * s) / bk) ** 2
        inside = q <= 1
        zk[inside] = z
        rk[inside] = r
        cov = np.maximum(cov, np.clip((1 - np.sqrt(q)) * bk + 0.5, 0, 1))
    vis = zk >= 0
    wx = (du / s + 2 * (dv + zk * s) / s) / 2
    wy = (2 * (dv + zk * s) / s - du / s) / 2
    phi = np.arctan2(wy, wx)
    slope = H * p * (np.maximum(rk, 1e-6) ** (p - 1)) / R ** p
    slope = np.where(rk <= rtop * 1.001, 0.0, slope)
    n = np.dstack([slope * np.cos(phi), slope * np.sin(phi), np.ones_like(slope)])
    n /= np.linalg.norm(n, axis=2, keepdims=True)
    diffuse = np.clip(n @ K.LIGHT, 0, 1)
    base = np.array([112, 84, 62], np.float32)
    grain = 1 + 0.12 * rng.standard_normal(du.shape).astype(np.float32)
    # Excavated soil darkens near the base where it is fresh.
    fresh = 0.85 + 0.15 * np.clip(zk / H, 0, 1)
    col = base * ((0.26 + 0.78 * diffuse) * grain * fresh)[..., None]
    a_ = (cov * vis)[..., None]
    rgb[y0:y1, x0:x1] = rgb[y0:y1, x0:x1] * (1 - a_) + col * a_
    alpha[y0:y1, x0:x1] = alpha[y0:y1, x0:x1] * (1 - a_[..., 0]) + a_[..., 0]
    # Crater entrance at the top, lit from inside in the process's kind colour.
    ex, ey = sx, sy - top
    ea = math.sqrt(2) * rtop * s * 0.8
    lay = K.Layer(x1 - x0, y1 - y0)
    lay.ellipse((ex - x0, ey - y0), (ea, ea / 2), 0, (14, 9, 7))
    lay.ellipse((ex - x0, ey - y0 + ea * 0.12), (ea * 0.7, ea * 0.3), 0, tuple(0.55 * np.array(kind_col)))
    # Side entrances on the slopes facing the viewer.
    for ang, frac in ((0.35, 0.28), (1.35, 0.4)):
        if R < 0.02 and frac > 0.3:
            continue
        zz = H * frac
        rr = R * (1 - zz / H) ** (1 / p)
        px, py = geo.p(x + rr * math.cos(ang) * 0.97, y + rr * math.sin(ang) * 0.97, zz)
        hw = max(2.0, 0.16 * a)
        lay.ellipse((px - x0, py - y0), (hw, hw * 0.62), 0, (14, 9, 7))
        lay.ellipse((px - x0, py - y0 + hw * 0.18), (hw * 0.62, hw * 0.3), 0, tuple(0.5 * np.array(kind_col)))
        glows.append((px, py, hw * 1.6, kind_col, 0.32))
    la = lay.a.astype(np.float32)[..., None] / 255.0
    rgb[y0:y1, x0:x1] = rgb[y0:y1, x0:x1] * (1 - la) + lay.rgb.astype(np.float32)
    alpha[y0:y1, x0:x1] = np.maximum(alpha[y0:y1, x0:x1], la[..., 0])
    glows.append((ex, ey, ea * 1.7, kind_col, 0.55))
    return sx, sy - top - b * 0.3


def render_pile(rgb, alpha, glows, geo, x, y, r, rng, fade=1.0, tint=(226, 214, 190)):
    s = geo.s
    a_px = math.sqrt(2) * r * s
    grain_r = max(1.6, min(0.11 * a_px, 3.2 * geo.ss))
    count = int(np.clip(1.5 * (a_px / grain_r) ** 2, 12, 420))
    hp = r * 0.75
    rho = r * np.sqrt(rng.random(count))
    th = rng.random(count) * 2 * np.pi
    gx, gy = x + rho * np.cos(th), y + rho * np.sin(th)
    gz = hp * (1 - rho / r) * rng.uniform(0.75, 1.0, count)
    order = np.argsort(gx + gy + gz)
    tint = np.array(tint, np.float32)
    for i in order:
        px, py = geo.p(gx[i], gy[i], gz[i])
        c = tint * rng.uniform(0.82, 1.05) * fade
        K.sphere_over(rgb, alpha, px, py, grain_r * rng.uniform(0.8, 1.15), c, spec_amt=0.35 * fade)
    sx, sy = geo.p(x, y)
    glows.append((sx, sy - hp * s * 0.4, a_px * 0.9, (255, 236, 190), 0.16 * fade))
    return sx, sy + a_px * 0.5


# ---------------------------------------------------------------------------------------------
# Frame rendering


def bezier(p0, p1, bend, n=64):
    p0, p1 = np.array(p0), np.array(p1)
    mid = (p0 + p1) / 2
    d = p1 - p0
    perp = np.array([-d[1], d[0]])
    c = mid + perp * bend
    t = np.linspace(0, 1, n)[:, None]
    return (1 - t) ** 2 * p0 + 2 * (1 - t) * t * c + t ** 2 * p1


class Renderer:
    def __init__(self, width, height, ss=2, tex_n=1024):
        self.W, self.H, self.ss = width, height, ss
        self.geo = K.Geo(width, height, ss, frac=0.86, top=0.07)
        self.tex_n = tex_n
        self.sand = sand_texture(tex_n)
        blanket(self.sand, tex_n)
        self.M = self.geo.texture_matrix(tex_n)
        wss, hss = width * ss, height * ss
        self.mask = cv2.warpAffine(np.ones((tex_n, tex_n), np.float32), self.M, (wss, hss),
                                   flags=cv2.INTER_LINEAR, borderValue=0)
        self.sky = S.sky(wss, hss)
        self.slab = self.make_slab()
        self.scale = width / 1600.0  # output px relative to the still

    def make_slab(self):
        g = self.geo
        wss, hss = self.W * self.ss, self.H * self.ss
        lay = K.Layer(wss, hss)
        th = 0.034
        L = [g.p(0, 1), g.p(1, 1), g.p(1, 1, -th), g.p(0, 1, -th)]
        R = [g.p(1, 1), g.p(1, 0), g.p(1, 0, -th), g.p(1, 1, -th)]
        lay.poly(L, (58, 44, 34))
        lay.poly(R, (36, 28, 24))
        # Strata lines in the cut faces.
        for frac, c in ((0.33, (76, 58, 44)), (0.62, (46, 34, 27)), (0.82, (64, 50, 38))):
            lay.line(g.p(0, 1, -th * frac), g.p(1, 1, -th * frac), c, 1.2 * self.ss)
            lay.line(g.p(1, 1, -th * frac), g.p(1, 0, -th * frac), tuple(0.7 * np.array(c)), 1.2 * self.ss)
        return lay

    def objects(self, t, rng_seed=5):
        """Mounds and piles, depth sorted, as a premultiplied layer plus glow list and anchors."""
        g = self.geo
        wss, hss = self.W * self.ss, self.H * self.ss
        rgb = np.zeros((hss, wss, 3), np.float32)
        alpha = np.zeros((hss, wss), np.float32)
        glows, anchors = [], {}
        items = []
        for name, (kind, x, y, mem, _, _) in NESTS.items():
            items.append((x + y, "nest", name))
        for key, (_, _, x, y, size) in FILES.items():
            if key != "tty":
                items.append((x + y, "file", key))
        for _, what, key in sorted(items):
            rng = np.random.default_rng(abs(hash(key)) % (2 ** 32) if False else sum(map(ord, key)) * 7 + rng_seed)
            if what == "nest":
                kind, x, y, mem, _, _ = NESTS[key]
                R = nest_radius(mem)
                anchors[key] = render_mound(rgb, alpha, glows, g, x, y, R, R * 0.95, S.KIND[kind], rng)
            else:
                _, _, x, y, size = FILES[key]
                fa = file_alpha(key, t)
                if fa <= 0.01:
                    continue
                r = pile_radius(size) * (0.4 + 0.6 * fa if key == "cco" else 1.0)
                anchors[key] = render_pile(rgb, alpha, glows, g, x, y, r, rng, fade=0.35 + 0.65 * fa)
                if fa < 1:
                    alpha *= 1.0
        return rgb, alpha, glows, anchors

    def frame(self, colony, t, labels=True, status_lines=(), events=()):
        g, ss = self.geo, self.ss
        wss, hss = self.W * ss, self.H * ss
        canvas = self.sky.copy()
        self.slab.onto(canvas)
        # Pheromone glow: kind-coloured, saturating, with a soft halo.
        P = colony.P
        dens = P.sum(0)
        chroma = P / (dens[None] + 1e-4)
        inten = 1 - np.exp(-dens * 2.2)
        trail = (chroma * inten[None]).transpose(1, 2, 0) * 255.0
        n = self.tex_n
        trail = cv2.resize(trail, (n, n), interpolation=cv2.INTER_CUBIC)
        halo = cv2.GaussianBlur(trail, (0, 0), n / 160)
        tex = self.sand + 0.70 * np.clip(trail, 0, 255) + 0.65 * halo
        ground = cv2.warpAffine(tex, self.M, (wss, hss), flags=cv2.INTER_LINEAR, borderValue=0)
        m = self.mask[..., None]
        canvas = canvas * (1 - m) + ground * m
        # Tunnels: dashed arcs between nests, marching in the direction of flow.
        tl = K.Layer(wss, hss)
        dots = []
        for i, (a, b, what) in enumerate(TUNNELS):
            ka, xa, ya = NESTS[a][0], NESTS[a][1], NESTS[a][2]
            xb, yb = NESTS[b][1], NESTS[b][2]
            pts = bezier((xa, ya), (xb, yb), 0.28 if i % 2 else -0.28)
            sp = np.array([g.p(px, py) for px, py in pts])
            seg = np.hypot(*np.diff(sp, axis=0).T)
            arc = np.r_[0, np.cumsum(seg)]
            dash = 9 * ss * self.scale
            phase = (arc - t * 30 * ss * self.scale) % (2 * dash)
            col = np.array(S.KIND[ka]) * 0.85 + 30
            for j in range(len(sp) - 1):
                if phase[j] < dash:
                    tl.line(sp[j], sp[j + 1], col, 2.2 * ss * self.scale + 0.5)
            for k in range(3):
                u = ((t * 0.35 + k / 3 + i * 0.17) % 1.0)
                j = min(int(u * (len(sp) - 1)), len(sp) - 2)
                dots.append((sp[j], col))
        tl.onto(canvas, 0.55)
        for (px, py), col in dots:
            S.glow(canvas, px, py, 3.2 * ss * self.scale, col, 0.7)
        # Ants and crumbs.
        pos, ang, fi, carrying = colony.ants()
        sx, sy = g.p(pos[:, 0], pos[:, 1])
        hx, hy = g.dirv(np.cos(ang), np.sin(ang))
        hn = np.hypot(hx, hy) + 1e-9
        hx, hy = hx / hn, hy / hn
        k = ss * self.scale * 1.05
        ants = np.zeros((hss, wss), np.uint8)
        crumbs_r = np.zeros((hss, wss), np.uint8)
        crumbs_w = np.zeros((hss, wss), np.uint8)
        sh = K.SHIFT
        one = K.ONE
        for i in range(len(sx)):
            x, y, dx, dy = sx[i], sy[i], hx[i], hy[i]
            for off, rad in ((-2.4, 1.45), (-0.5, 0.95), (1.1, 1.05)):
                cv2.circle(ants, (int((x + dx * off * k) * one), int((y + dy * off * k) * one)),
                           max(1, int(rad * k * one)), 255, -1, cv2.LINE_AA, sh)
            if carrying[i]:
                f = FLOWS[fi[i]]
                nest = NESTS[f[0]]
                nbytes = nest[5] if f[2] == "w" else nest[4]
                cr = crumb_px(nbytes) * ss * self.scale
                cx, cy = x + dx * (2.3 * k + cr * 0.8), y + dy * (2.3 * k + cr * 0.8)
                target = crumbs_w if f[2] == "w" else crumbs_r
                cv2.circle(target, (int(cx * one), int(cy * one)), max(1, int(cr * one)), 255, -1,
                           cv2.LINE_AA, sh)
        am = ants.astype(np.float32)[..., None] / 255.0
        canvas = canvas * (1 - 0.92 * am) + ANT * 0.92 * am
        for cm, col in ((crumbs_r, READ_CRUMB), (crumbs_w, WRITE_CRUMB)):
            c = cm.astype(np.float32)[..., None] / 255.0
            canvas = canvas * (1 - c) + col * c
            canvas += cv2.GaussianBlur(c[..., 0], (0, 0), 2.6 * ss * self.scale)[..., None] * col * 0.30
        # Mounds and food piles over the ants (ants vanish into the nest).
        rgb, alpha, glows, anchors = self.objects(t)
        K.composite(canvas, rgb, alpha)
        for px, py, rad, col, st in glows:
            S.glow(canvas, px, py, rad, col, st)
        img = K.downsample(canvas, self.W, self.H)
        if labels:
            self.labels(img, anchors, t, events, colony)
        S.status(img, list(status_lines), size=self.status_size)
        return img

    status_size = None

    def labels(self, img, anchors, t, events, colony):
        draw = ImageDraw.Draw(img)
        g, ss, sc = self.geo, self.ss, self.scale
        big, mid, small = max(10, round(14 * sc)), max(10, round(13 * sc)), max(9, round(12 * sc))
        compact = self.W < 1200
        if compact:
            big, mid, small = 13, 12, 11
        for name, (x0, y0, x1, y1), _ in REGIONS:
            px, py = g.p(x0 + 0.035, y0 + 0.035) if name != "/var/lib" else g.p(x0 + 0.06, y1 - 0.035)
            S.label(draw, px / ss, py / ss, name, size=big, fill=(214, 200, 172), anchor="mm")
        for name, (kind, x, y, mem, _, _) in NESTS.items():
            ax, ay = anchors[name]
            mem_s = f"{mem / 1024:.1f} GiB" if mem >= 1024 else f"{mem} MiB"
            if compact and name == "nginx-128":
                S.label(draw, ax / ss + 16, ay / ss + 6, name, size=mid, fill=S.TEXT, anchor="lm")
            elif compact:
                S.label(draw, ax / ss, ay / ss - 9, name, size=mid, fill=S.TEXT)
            else:
                S.label(draw, ax / ss, ay / ss - 22 * sc, name, size=mid, fill=S.TEXT)
                S.label(draw, ax / ss, ay / ss - 8 * sc, mem_s, size=small, fill=S.DIM)
        for key, (_, lab, x, y, size) in FILES.items():
            if key == "tty":
                px, py = g.p(x, y)
                S.label(draw, px / ss, py / ss + 34 * sc, lab, size=mid, fill=(236, 214, 200))
                continue
            if key not in anchors:
                continue
            if compact and key not in ("libc", "llvm", "journal", "access", "pgbase", "buildlog", "cco", "mainrs"):
                continue
            fa = file_alpha(key, t)
            ax, ay = anchors[key]
            fill = tuple(int(c * (0.45 + 0.55 * fa)) for c in (226, 218, 196))
            S.label(draw, ax / ss, ay / ss + 9 * sc, lab, size=small, fill=fill)
        # Crumb-size callouts on two contrasting trails.
        for nest, key, text, side, u, dy in (("worker-134", "access", "64 B/read", -1, 0.40, 32),
                                             ("compiler-90", "llvm", "128 KiB/read", 1, 0.85, 30)):
            fi = [i for i, f in enumerate(FLOWS) if f[0] == nest and f[1] == key][0]
            p0, c1, p1 = colony.npos[fi], colony.ctrl[fi], colony.fpos[fi]
            mx, my = (1 - u) ** 2 * p0 + 2 * (1 - u) * u * c1 + u ** 2 * p1
            px, py = g.p(mx, my)
            px, py = px / ss, py / ss
            ox, oy = 46 * sc * side, dy * sc
            draw.line([(px, py), (px + ox * 0.82, py + oy * 0.82)], fill=(200, 200, 210), width=1)
            S.label(draw, px + ox, py + oy, text, size=mid, fill=(255, 240, 205),
                    anchor="lm" if side > 0 else "rm")
        for (x, y, text, col) in events:
            px, py = g.p(x, y)
            S.label(draw, px / ss, py / ss, text, size=mid, fill=col)


def status_lines(colony, t, wide):
    n = int(colony.alive.sum())
    first = (f"ISOTOP / ANTHILL / DEMO   192 processes | 10 nests | 13 open files | {n:,} ants   "
             f"K = 1 ant per 50 syscalls  syscr 48,210/s  syscw 9,377/s")
    if wide:
        return [first,
                "mound = process (size ~ memory) | pile = open file on a treemap of / | ant = K syscalls | "
                "crumb size = bytes per syscall | pale crumb = read, coral = write | glow = pheromone | dashes = pipes, sockets",
                f"Tab next view | v views | click a nest or pile to inspect | Space pause | q quit   "
                f"fdinfo: top 24 by syscalls | 22,610 syscalls/s via tunnels | t={42 + t:.1f}s"]
    return [first.replace("   K =", " | K =").replace("ISOTOP / ANTHILL / DEMO   192 processes | ", "ISOTOP / ANTHILL / DEMO   192 procs | "),
            "mound = process (~memory) | pile = open file, treemap of / | ant = K syscalls | crumb = bytes/syscall",
            "pale crumb = read, coral = write | glow = pheromone | dashes = pipes, sockets | "
            f"t={42 + t:.1f}s"]


def make_still():
    col = Colony()
    col.run(12.0)
    col.t = T_OPEN + 1.6  # (the colony clock keeps running on from here)
    col.run(0.0)
    r = Renderer(1600, 900, ss=2, tex_n=1536)
    img = r.frame(col, col.t, status_lines=status_lines(col, col.t, True))
    return img


def main(which):
    if which in ("still", "both"):
        # Warm up with the scenario clock so trails exist; stop shortly after the new file opens.
        col = Colony()
        col.t = -12.0
        col.run(12.0 + T_OPEN + 1.9)
        r = Renderer(1600, 900, ss=2, tex_n=1536)
        r.status_size = 12
        events = [(FILES["cco"][2] + 0.055, FILES["cco"][3] + 0.055, "new: opened by compiler-90", (255, 236, 190))]
        img = r.frame(col, col.t, status_lines=status_lines(col, col.t, True), events=events)
        print("still", S.save_still(img, "anthill"), int(col.alive.sum()), "ants")
    if which in ("anim", "both"):
        col = Colony()
        col.t = -12.0
        col.run(12.0)
        r = Renderer(960, 540, ss=2, tex_n=1024)
        fps, seconds = 12, 10
        frames = []
        for i in range(fps * seconds):
            t = i / fps
            events = []
            if T_OPEN <= t < T_OPEN + 4.5:
                events.append((FILES["cco"][2] + 0.06, FILES["cco"][3] + 0.06, "opened by compiler-90", (255, 236, 190)))
            if T_CLOSE <= t < T_CLOSE + 4.0:
                events.append((FILES["cache"][2] - 0.045, FILES["cache"][3] - 0.045, "closed", (255, 170, 150)))
            frames.append(r.frame(col, t, status_lines=status_lines(col, t, False), events=events))
            col.run(1 / fps, dt=1 / 24)
            if i % 20 == 0:
                print("frame", i, int(col.alive.sum()), flush=True)
        path, size = K.save_animation(frames, "anthill", fps=fps)
        K.contact_sheet([frames[i] for i in (6, 40, 75, 112)], os.path.join(S.OUT, "anthill-sheet.png"))
        print("gif", path, f"{size / 1e6:.2f} MB")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "both")
