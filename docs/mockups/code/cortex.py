"""Mockup of isotop's `cortex` view: a stained slice of cortex where every process is a neuron.

Real simulation in the mockup:
- Izhikevich 2003 neurons, v' = 0.04 v^2 + 5 v + 140 - u + I, u' = a (b v - u), reset at
  v >= 30 mV, integrated in 0.5 ms steps. Kind picks the parameter set: kernel FS, system RS,
  session CH, containers IB.
- Input current is a monotonic, per-type calibrated function of CPU (idle silent, ~5% CPU at
  rheobase so it fires occasionally, one full core ~40 Hz), plus a little current noise.
- Socket links are excitatory synapses with weight ~ log(traffic) and a conduction delay
  proportional to the on-screen axon length; pulses on the axons are those very spikes in
  flight. Busy processes that talk pull each other into step.
- Synchrony is Golomb's chi computed from the simulated membrane potentials; the raster, the
  inspector trace and the firing-type cards all read the same run.
Mocked: the process table and CPU curves (a web request burst at t = 3 s), the dendrite
shapes (seeded, one primary branch per thread) and the stain texture.

usage: python3 -I cortex.py [still|anim|both]
"""
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import cv2  # noqa: E402
import numpy as np  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402

import isostyle as S  # noqa: E402
import mockkit as K  # noqa: E402

DT = 0.5                      # ms, as in the paper
T0, T1 = -11.0, 11.0          # simulated seconds
PARAMS = {"kernel": (0.1, 0.2, -65.0, 2.0), "system": (0.02, 0.2, -65.0, 8.0),
          "session": (0.02, 0.2, -50.0, 2.0), "container": (0.02, 0.2, -55.0, 4.0)}
TYPE_NAME = {"kernel": ("FS", "fast spiking"), "system": ("RS", "regular spiking"),
             "session": ("CH", "chattering"), "container": ("IB", "intrinsically bursting")}
LAYERS = [  # name, kind, y0, y1 (design px)
    ("I", None, 40, 92),
    ("II/III", "session", 92, 222),
    ("IV", "container", 222, 318),
    ("V", "system", 318, 448),
    ("VI", "kernel", 448, 556),
]
WM = (556, 596)
SLICE_X = (150, 1192)
RASTER = (150, 628, 1580, 812)   # x0, y0, x1, y1
NOISE = 1.2                     # current noise per step (also used in the calibration)
CONDUCTION = 1.25               # px per ms on screen, so delay ~ axon length


# ---------------------------------------------------------------------------- process table
def table():
    rng = np.random.default_rng(11)
    N = []

    def add(name, kind, cpu, threads, mem, pos=None, label=False):
        N.append(dict(name=name, kind=kind, cpu=cpu, threads=threads, mem=mem, pos=pos, label=label))

    # Session, layer II/III (chattering).
    add("compiler-90", "session", 1.0, 12, 1.2, (330, 168), True)
    add("language-server-71", "session", 0.32, 14, 0.9, (205, 140), True)
    add("code-201", "session", 0.12, 31, 1.4, (150 + 120, 120))
    add("browser-512", "session", 0.30, 48, 3.1, (700, 150), True)
    for i, (x, y, c) in enumerate(((800, 120, 0.22), (868, 176, 0.12), (760, 196, 0.30))):
        add(f"browser-{513 + i}", "session", c, 18, 0.6, (x, y))
    add("pipewire-95", "session", 0.08, 4, 0.1, (590, 118), True)
    add("gnome-shell-88", "session", 0.16, 22, 0.8, (1010, 150), True)
    add("shell-170", "session", 0.0, 1, 0.01, (470, 130), True)
    for i in range(12):
        add(f"user-{600 + i}", "session", float(rng.choice([0, 0, 0, 0, 0.01, 0.03, 0.06])), 3, 0.1)
    # Containers, layer IV (intrinsically bursting): the web stack.
    add("nginx-128", "container", 0.22, 8, 0.1, (560, 262), True)
    for i, (x, y) in enumerate(((650, 296), (728, 252), (812, 292), (890, 250))):
        add(f"worker-{134 + i}", "container", 0.18 + 0.03 * i, 16, 0.4, (x, y), i == 0)
    add("postgres-41", "container", 0.30, 9, 2.2, (995, 284), True)
    add("redis-207", "container", 0.12, 4, 0.6, (1095, 256), True)
    for i in range(7):
        add(f"shim-{700 + i}", "container", float(rng.choice([0, 0, 0.01, 0.02])), 10, 0.02)
    # System, layer V (regular spiking).
    add("systemd-1", "system", 0.04, 1, 0.05, (470, 392), True)
    add("journald-310", "system", 0.07, 2, 0.2, (330, 360), True)
    add("dbus-102", "system", 0.05, 2, 0.02, (640, 410))
    add("NetworkManager-640", "system", 0.02, 4, 0.03, (830, 372))
    for i in range(16):
        add(f"svc-{300 + i}", "system", float(rng.choice([0, 0, 0, 0, 0, 0.02, 0.05])), 3, 0.02)
    # Kernel, layer VI (fast spiking).
    add("irq/142-nvme0q3", "kernel", 0.30, 1, 0.0, (920, 500), True)
    add("kworker/3:1", "kernel", 0.12, 1, 0.0, (690, 486))
    add("ksoftirqd/2", "kernel", 0.09, 1, 0.0, (430, 512))
    for i in range(27):
        add(f"kthread-{i}", "kernel", float(rng.choice([0, 0, 0, 0, 0, 0.02, 0.05, 0.08])), 1, 0.0)
    return N


LINKS = [  # a, b, traffic (bytes/s)
    ("nginx-128", "worker-134", 9e5), ("nginx-128", "worker-135", 8e5),
    ("nginx-128", "worker-136", 8e5), ("nginx-128", "worker-137", 7e5),
    ("worker-134", "postgres-41", 4e5), ("worker-135", "postgres-41", 3e5),
    ("worker-136", "postgres-41", 3e5), ("worker-137", "postgres-41", 2e5),
    ("worker-134", "redis-207", 2e5), ("worker-136", "redis-207", 1.5e5),
    ("browser-512", "browser-513", 3e5), ("browser-512", "browser-514", 1e5),
    ("browser-512", "browser-515", 2e5), ("browser-512", "pipewire-95", 6e4),
    ("browser-512", "nginx-128", 1.2e5),
    ("code-201", "language-server-71", 5e4), ("language-server-71", "compiler-90", 2e4),
    ("systemd-1", "journald-310", 3e3), ("dbus-102", "systemd-1", 2e3),
    ("NetworkManager-640", "dbus-102", 1e3), ("gnome-shell-88", "dbus-102", 4e3),
    ("pipewire-95", "gnome-shell-88", 2e4), ("nginx-128", "journald-310", 4e3),
    ("postgres-41", "journald-310", 1e3),
]
WEB = ["nginx-128", "worker-134", "worker-135", "worker-136", "worker-137", "postgres-41", "redis-207"]


def place(N):
    """Named neurons have fixed homes; the rest are scattered in their layer, kept apart."""
    rng = np.random.default_rng(5)
    taken = [n["pos"] for n in N if n["pos"]]
    band = {k: (y0, y1) for _, k, y0, y1 in LAYERS if k}
    for n in N:
        if n["pos"]:
            continue
        y0, y1 = band[n["kind"]]
        for _ in range(4000):
            p = (rng.uniform(SLICE_X[0] + 30, SLICE_X[1] - 25), rng.uniform(y0 + 16, y1 - 14))
            if all((p[0] - q[0]) ** 2 + (p[1] - q[1]) ** 2 > 40 ** 2 for q in taken):
                break
        n["pos"] = p
        taken.append(p)
    for n in N:
        n["z"] = float(rng.uniform(0.0, 1.0)) if not n["label"] else 1.0


def bezier(p0, p3, rng, sag):
    p0, p3 = np.array(p0, float), np.array(p3, float)
    p1 = p0 + np.array([rng.uniform(-20, 20), sag])
    p2 = p3 + np.array([rng.uniform(-20, 20), sag])
    t = np.linspace(0, 1, 160)[:, None]
    pts = (1 - t) ** 3 * p0 + 3 * (1 - t) ** 2 * t * p1 + 3 * (1 - t) * t * t * p2 + t ** 3 * p3
    seg = np.linalg.norm(np.diff(pts, axis=0), axis=1)
    return pts, np.concatenate([[0], np.cumsum(seg)])


# ---------------------------------------------------------------------------- simulation
def cpu_at(n, t):
    c = n["cpu"]
    if n["name"] in WEB:
        # A burst of web requests: the whole stack gets busy for ~0.7 s.
        burst = math.exp(-((t - 3.0) / 0.32) ** 2)
        c = c + 1.5 * burst * (1.0 if n["name"] != "redis-207" else 0.5)
    if n["name"] == "compiler-90":
        c = 0.93 + 0.03 * math.sin(t * 1.3)
    return c


def calibrate():
    """Per type: rheobase and the current that gives 40 Hz, from a quick f-I sweep."""
    I = np.linspace(0, 30, 241)
    rng = np.random.default_rng(4)
    out = {}
    for kind, (a, b, c, d) in PARAMS.items():
        v = np.full(I.size, -65.0)
        u = b * v
        n = np.zeros(I.size)
        for s in range(int(3000 / DT)):
            fired = v >= 30
            if s * DT > 1000:
                n += fired
            v = np.where(fired, c, v)
            u = np.where(fired, u + d, u)
            v = v + DT * (0.04 * v * v + 5 * v + 140 - u + I + rng.normal(0, NOISE, I.size))
            u = u + DT * a * (b * v - u)
        r = n / 2.0
        r = np.maximum.accumulate(r)
        out[kind] = (float(I[np.argmax(r >= 1.0)]), float(np.interp(40, r, I)))
    return out


def current(kind, cpu, cal):
    rh, i40 = cal[kind]
    # Monotonic: 0 -> 0, 0.5% -> well below rheobase, 5% -> just over it, 1 core -> 40 Hz.
    xs = [0.0, 0.005, 0.05, 1.0, 3.0]
    ys = [0.0, 0.62 * rh, 0.98 * rh, i40, i40 + 2.0 * (i40 - 0.98 * rh)]
    return np.interp(cpu, xs, ys)


class Cortex:
    def __init__(self):
        self.N = N = table()
        place(N)
        self.idx = {n["name"]: i for i, n in enumerate(N)}
        self.n = len(N)
        self.kind = [n["kind"] for n in N]
        a = np.array([PARAMS[k][0] for k in self.kind])
        b = np.array([PARAMS[k][1] for k in self.kind])
        c = np.array([PARAMS[k][2] for k in self.kind])
        d = np.array([PARAMS[k][3] for k in self.kind])
        self.abcd = a, b, c, d
        self.cal = calibrate()
        # Axons: one drawn curve per socket link, spikes run both ways along it.
        rng = np.random.default_rng(3)
        self.axons = []
        maxlog = math.log1p(max(tr for *_, tr in LINKS))
        for an, bn, tr in LINKS:
            ia, ib = self.idx[an], self.idx[bn]
            pa, pb = np.array(N[ia]["pos"]), np.array(N[ib]["pos"])
            sag = 34 + 0.12 * abs(pa[0] - pb[0])
            pts, cum = bezier(pa + (0, 8), pb + (0, 8), rng, sag)
            L = cum[-1]
            w = 1.7 * math.log1p(tr) / maxlog
            self.axons.append(dict(a=ia, b=ib, pts=pts, cum=cum, delay=L / CONDUCTION, w=w, traffic=tr))

    def run(self):
        n = self.n
        a, b, c, d = self.abcd
        steps = int(round((T1 - T0) * 1000 / DT))
        self.steps = steps
        v = np.full(n, -65.0) + np.random.default_rng(1).uniform(-5, 5, n)
        u = b * v
        isyn = np.zeros(n)
        src, dst, dl, w = [], [], [], []
        for ax in self.axons:
            for s_, t_ in ((ax["a"], ax["b"]), (ax["b"], ax["a"])):
                src.append(s_)
                dst.append(t_)
                dl.append(int(round(ax["delay"] / DT)))
                w.append(ax["w"])
        src, dst, dl, w = map(np.array, (src, dst, dl, w))
        D = dl.max() + 2
        buf = np.zeros((D, n))
        rng = np.random.default_rng(2)
        self.V = np.zeros((steps, n), np.float32)
        spikes = [[] for _ in range(n)]
        tau_syn = 5.0
        # CPU sampled at the sample tick rate (100 ms) like isotop's smoothed CPU.
        Icache = {}
        for s in range(steps):
            t = T0 + s * DT / 1000.0
            key = int(math.floor(t * 10))
            if key not in Icache:
                Icache.clear()
                tt = key / 10.0
                Icache[key] = np.array([current(nn["kind"], cpu_at(nn, tt), self.cal) for nn in self.N])
            I_ext = Icache[key]
            slot = s % D
            isyn = isyn * math.exp(-DT / tau_syn) + buf[slot]
            buf[slot] = 0
            active = I_ext > 0
            noise = rng.normal(0, NOISE, n) * active
            fired = v >= 30
            if fired.any():
                for i in np.nonzero(fired)[0]:
                    spikes[i].append(t)
                sel = fired[src]
                if sel.any():
                    np.add.at(buf, ((s + dl[sel]) % D, dst[sel]), w[sel])
            v = np.where(fired, c, v)
            u = np.where(fired, u + d, u)
            I = I_ext + isyn + noise
            v = v + DT * (0.04 * v * v + 5 * v + 140 - u + I)
            u = u + DT * a * (b * v - u)
            v = np.minimum(v, 30.0 + 1e-3)   # the peak; also keeps the record readable
            self.V[s] = np.where(fired, 30.0, v)
        self.spikes = [np.array(sp) for sp in spikes]

    def sidx(self, t):
        return int(np.clip(round((t - T0) * 1000 / DT), 0, self.steps - 1))

    def chi(self, members, t, window=2.0):
        """Golomb's synchrony measure over the last `window` seconds."""
        s1, s0 = self.sidx(t), self.sidx(t - window)
        V = self.V[s0:s1, members].astype(np.float64)
        if V.shape[0] < 10:
            return 0.0
        vi = V.var(0)
        vm = V.mean(1).var()
        return float(math.sqrt(vm / max(vi.mean(), 1e-9)))

    def rate(self, i, t, window=2.0):
        sp = self.spikes[i]
        return float(((sp > t - window) & (sp <= t)).sum() / window)


# ---------------------------------------------------------------------------- drawing
def dendrites(n, rng):
    """Seeded Golgi-stain-like tree: apical trunk to layer I with a tuft, one primary basal
    branch per thread (capped), each bifurcating. Returns a list of polylines."""
    x, y = n["pos"]
    lines = []

    def wiggle(p0, ang, length, steps=6, curl=0.18):
        pts = [p0]
        p = np.array(p0, float)
        for _ in range(steps):
            ang += rng.normal(0, curl)
            p = p + length / steps * np.array([math.cos(ang), math.sin(ang)])
            pts.append(p.copy())
        return pts, ang

    def branch(p0, ang, length, depth):
        pts, a2 = wiggle(p0, ang, length)
        lines.append((pts, depth))
        if depth < 2 and length > 10:
            for da in (-0.45, 0.45):
                branch(pts[-1], a2 + da + rng.normal(0, 0.15), length * rng.uniform(0.55, 0.75), depth + 1)

    if n["kind"] == "kernel":
        k = 5 + int(rng.integers(0, 3))
        for j in range(k):
            ang = 2 * math.pi * j / k + rng.normal(0, 0.3)
            branch((x, y), ang, rng.uniform(10, 20), 1)
        return lines
    # Apical dendrite up to layer I.
    top = 50 + rng.uniform(0, 18)
    trunk, ang = wiggle((x, y - 8), -math.pi / 2, y - 8 - top, steps=10, curl=0.05)
    lines.append((trunk, 0))
    for j in range(3):
        branch(trunk[-1], -math.pi / 2 + (j - 1) * 0.9 + rng.normal(0, 0.2), rng.uniform(14, 26), 1)
    for k in range(2, len(trunk) - 2, 3):
        side = 1 if rng.random() < 0.5 else -1
        branch(trunk[k], -math.pi / 2 + side * rng.uniform(0.7, 1.1), rng.uniform(10, 18), 2)
    nb = int(np.clip(n["threads"], 3, 9))
    for j in range(nb):
        ang = math.pi * (0.1 + 0.8 * (j + rng.uniform(-0.3, 0.3)) / max(nb - 1, 1))
        branch((x, y + 2), ang, rng.uniform(14, 28), 1)
    return lines


def soma_poly(x, y, r, kind):
    if kind == "kernel":
        t = np.linspace(0, 2 * math.pi, 28)
        return np.stack([x + r * 0.8 * np.cos(t), y + r * 0.8 * np.sin(t)], 1)
    # Teardrop (pyramidal cell) pointing up: rounded base, apex toward the pia.
    t = np.linspace(0, 2 * math.pi, 48)
    px = r * np.sin(t) * (0.55 + 0.45 * np.sin(t / 2) ** 1.0)
    py = -r * np.cos(t) * 1.25 + r * 0.25
    px = np.sin(t) * r * np.sin(t / 2) ** 0.9
    py = -np.cos(t) * r * 1.35 + r * 0.35
    return np.stack([x + px, y + py], 1)


class Renderer:
    def __init__(self, cx, w, h, ss=2):
        self.cx, self.w, self.h, self.ss = cx, w, h, ss
        self.s = w / 1600.0
        self.k = self.s * ss
        self.base = self.make_base()

    def P(self, x, y):
        return x * self.k, y * self.k

    def make_base(self):
        W, H, k = self.w * self.ss, self.h * self.ss, self.k
        sky = S.sky(self.w, self.h, glow=(120, 52, 128), base=(8, 7, 17), centre=(0.10, 0.22))
        canvas = cv2.resize(sky, (W, H), interpolation=cv2.INTER_LINEAR)
        yy, xx = np.mgrid[0:H, 0:W].astype(np.float32) / k
        # The slice: a faint warm tissue tint with soft edges, a little paler in the granular IV.
        x0, x1 = SLICE_X
        edge = (np.clip((xx - (x0 - 20)) / 40, 0, 1) * np.clip(((x1 + 10) - xx) / 30, 0, 1)
                * np.clip((yy - 30) / 20, 0, 1) * np.clip((WM[1] + 8 - yy) / 20, 0, 1))
        rng = np.random.default_rng(9)
        n = K.value_noise(512, 10, rng, ((1, 1), (3, 0.5)))
        n = cv2.resize(n, (W, H), interpolation=cv2.INTER_CUBIC)
        tissue = (10 + 3 * n)[..., None] * np.array([1.0, 0.75, 0.95], np.float32)
        for name, kind, y0, y1 in LAYERS:
            if name in ("IV",):
                tissue = tissue + (((yy > y0) & (yy < y1)) * 4.0)[..., None]
        canvas += tissue * edge[..., None]
        # Nissl stipple: many tiny dim cell bodies, densest in II/III and IV (decorative).
        lay = K.Layer(W, H)
        dens = {"I": 0.15, "II/III": 1.0, "IV": 1.3, "V": 0.7, "VI": 0.9}
        for name, kind, y0, y1 in LAYERS:
            m = int(dens[name] * (x1 - x0) * (y1 - y0) / 260)
            px = rng.uniform(x0, x1, m)
            py = rng.uniform(y0, y1, m)
            for a, b_ in zip(px, py):
                c = rng.uniform(16, 30)
                lay.circle((a * k, b_ * k), rng.uniform(0.6, 1.3) * k, (c, c * 0.8, c * 1.05))
        lay.onto(canvas, 0.8)
        # White matter: long faint fibres.
        lay = K.Layer(W, H)
        for j in range(26):
            yb = rng.uniform(WM[0] + 2, WM[1] - 4)
            xs = np.linspace(x0 - 10, x1 + 5, 60)
            ys = yb + 3 * np.sin(xs / rng.uniform(60, 140) + rng.uniform(0, 6))
            lay.polyline(np.stack([xs, ys], 1) * k, (60, 48, 70), 1)
        lay.onto(canvas, 0.6)
        # Layer boundaries.
        lay = K.Layer(W, H)
        for name, kind, y0, y1 in LAYERS[1:] + [("WM", None, WM[0], WM[1])]:
            for xa in np.arange(x0, x1, 14):
                lay.line((xa * k, y0 * k), ((xa + 6) * k, y0 * k), (70, 66, 92), 1)
        lay.onto(canvas, 0.7)
        # Dendrites, far cells softer (depth of field).
        far, near = K.Layer(W, H), K.Layer(W, H)
        for i, nn in enumerate(self.cx.N):
            rng2 = np.random.default_rng(1000 + i)
            col = np.array(S.KIND[nn["kind"]], np.float32)
            dim = 0.42 + 0.3 * nn["z"]
            L = near if nn["z"] > 0.45 else far
            for pts, depth in dendrites(nn, rng2):
                arr = np.array(pts) * k
                L.polyline(arr, tuple(col * dim * (1.0 - 0.18 * depth)), 1.15 * k if depth == 0 else 0.8 * k)
        fa = far.rgb.astype(np.float32)
        fa = cv2.GaussianBlur(fa, (0, 0), 0.9 * k)
        canvas += fa * 0.55
        near.add_onto(canvas, 0.75)
        # Axons: faint paths along socket links.
        lay = K.Layer(W, H)
        for ax in self.cx.axons:
            col = np.array(S.KIND[self.cx.N[ax["a"]]["kind"]], np.float32) * 0.5 + 40
            lay.polyline(ax["pts"] * k, tuple(col * 0.7), max(1.0, (0.6 + 0.25 * ax["w"]) * k))
        lay.onto(canvas, 0.55)
        # Panels: raster and the right-hand column.
        rx0, ry0, rx1, ry1 = RASTER
        K.blend_poly(canvas, np.array([[rx0 - 6, ry0 - 22], [rx1 + 6, ry0 - 22], [rx1 + 6, ry1 + 6],
                                       [rx0 - 6, ry1 + 6]]) * k, (9, 9, 15), 0.88)
        for (bx0, by0, bx1, by1) in self.boxes():
            K.blend_poly(canvas, np.array([[bx0, by0], [bx1, by0], [bx1, by1], [bx0, by1]]) * k,
                         (11, 10, 19), 0.86)
        lay = K.Layer(W, H)
        for (bx0, by0, bx1, by1) in self.boxes() + [(rx0 - 6, ry0 - 22, rx1 + 6, ry1 + 6)]:
            lay.polyline(np.array([[bx0, by0], [bx1, by0], [bx1, by1], [bx0, by1]]) * k, (58, 58, 82), 1,
                         closed=True)
        # Raster grid: one line per second, layer separators.
        for sec in range(11):
            X = rx0 + (rx1 - rx0) * sec / 10
            lay.line((X * k, ry0 * k), (X * k, ry1 * k), (30, 30, 44), 1)
        for yb in self.raster_bounds()[1:-1]:
            lay.line((rx0 * k, yb * k), (rx1 * k, yb * k), (40, 40, 58), 1)
        lay.onto(canvas)
        return canvas

    def boxes(self):
        return [(1212, 26, 1580, 232), (1212, 246, 1580, 596)]

    def raster_order(self):
        if hasattr(self, "_order"):
            return self._order
        cx = self.cx
        order = []
        for _, kind, *_ in LAYERS[1:]:
            ids = [i for i, n in enumerate(cx.N) if n["kind"] == kind and cx.spikes[i].size]
            # Busy first, then by x so linked neighbours sit together.
            ids.sort(key=lambda i: (-(cx.N[i]["cpu"] > 0.03), cx.N[i]["pos"][0]))
            order += ids
        self._order = order
        return order

    def raster_bounds(self):
        rx0, ry0, rx1, ry1 = RASTER
        cx = self.cx
        n = len(self.raster_order())
        bounds = [ry0]
        cnt = 0
        for _, kind, *_ in LAYERS[1:]:
            cnt += sum(1 for i in self.raster_order() if cx.N[i]["kind"] == kind)
            bounds.append(ry0 + (ry1 - ry0) * cnt / n)
        return bounds

    # -------------------------------------------------------------- per frame
    def frame(self, t, sel="compiler-90", births=()):
        cx, k = self.cx, self.k
        canvas = self.base.copy()
        W, H = canvas.shape[1], canvas.shape[0]
        # Pulses in flight along the axons.
        glow = np.zeros_like(canvas)
        for ax in cx.axons:
            L = ax["cum"][-1]
            for s_, fwd in ((ax["a"], True), (ax["b"], False)):
                sp = cx.spikes[s_]
                dly = ax["delay"] / 1000.0
                inflight = sp[(sp > t - dly) & (sp <= t)]
                if inflight.size == 0:
                    continue
                col = np.array(S.KIND[cx.N[s_]["kind"]], np.float32) * 0.6 + 100
                for ts in inflight:
                    f = (t - ts) / dly
                    for lag, amp in ((0.0, 1.0), (0.035, 0.45), (0.07, 0.2)):
                        ff = f - lag
                        if ff < 0:
                            continue
                        dist = (ff if fwd else 1 - ff) * L
                        j = min(int(np.searchsorted(ax["cum"], dist)), len(ax["pts"]) - 1)
                        x, y = ax["pts"][j]
                        S.glow(glow, x * k, y * k, 2.2 * k, col, 0.55 * amp * (0.5 + 0.12 * ax["w"]))
        canvas += glow
        # Somas: flash on spikes, glow halo.
        lay = K.Layer(W, H)
        halo = np.zeros_like(canvas)
        for i, n in enumerate(cx.N):
            x, y = n["pos"]
            sp = cx.spikes[i]
            rec = sp[(sp > t - 0.6) & (sp <= t)]
            flash = float(np.exp(-(t - rec) / 0.07).sum()) if rec.size else 0.0
            flash = min(flash, 1.6)
            act = min(cx.rate(i, t, 1.0) / 40.0, 1.0)
            r = (4.2 + 2.2 * min(n["mem"], 3) ** 0.5) * (0.75 + 0.25 * n["z"])
            if n["kind"] == "kernel":
                r = 3.6 + 0.6 * n["z"]
            grow = 1.0
            for name, tb in births:
                if name == n["name"]:
                    grow = float(np.clip((t - tb) / 0.6, 0, 1))
            if grow <= 0:
                continue
            r *= grow
            col = np.array(S.KIND[n["kind"]], np.float32)
            bright = (0.32 + 0.25 * n["z"] + 0.15 * (n["kind"] == "kernel")) + 0.25 * act + 0.55 * min(flash, 1.0)
            c = np.clip(col * bright + 255 * 0.35 * max(flash - 0.6, 0), 0, 255)
            lay.poly(soma_poly(x, y, r, n["kind"]) * k, tuple(c))
            lay.circle((x * k, (y + (0 if n["kind"] == "kernel" else r * 0.12)) * k), r * 0.32 * k,
                       tuple(np.clip(c * 0.55 + 70 * min(flash, 1), 0, 255)))
            if flash > 0.05 or act > 0.1:
                S.glow(halo, x * k, y * k, (7 + 5 * min(flash, 1)) * k, col,
                       0.10 * act + 0.45 * min(flash, 1.0))
        canvas += halo * 0.9
        lay.onto(canvas)
        # Selection ring around the inspected neuron.
        lay = K.Layer(W, H)
        si = cx.idx[sel]
        sx, sy = cx.N[si]["pos"]
        lay.circle((sx * k, (sy - 2) * k), 15 * k, (235, 235, 245), max(1, 1.0 * k))
        lay.onto(canvas, 0.85)
        # Raster.
        rx0, ry0, rx1, ry1 = RASTER
        order = self.raster_order()
        rowh = (ry1 - ry0) / len(order)
        lay = K.Layer(W, H)
        for row, i in enumerate(order):
            sp = cx.spikes[i]
            sp = sp[(sp > t - 10) & (sp <= t)]
            if sp.size == 0:
                continue
            col = np.array(S.KIND[cx.N[i]["kind"]], np.float32)
            yc = ry0 + (row + 0.5) * rowh
            xs = rx0 + (sp - (t - 10)) / 10 * (rx1 - rx0)
            c = tuple(np.clip(col * 1.15 + 20, 0, 255))
            for X in xs:
                lay.line((X * k, (yc - rowh * 0.42) * k), (X * k, (yc + rowh * 0.42) * k), c, max(1, 1.1 * k))
        lay.onto(canvas)
        # Braille traces in the inspector and the firing-type cards (drawn as terminal dots).
        lay = K.Layer(W, H)
        self.traces = []
        self.braille(lay, si, t, 0.30, 1224, 98, 46, 4, (235, 220, 150))
        self.cards = []
        for j, kind in enumerate(("system", "session", "container", "kernel")):
            ids = [i for i, n in enumerate(cx.N) if n["kind"] == kind]
            rep = max(ids, key=lambda i: cx.rate(i, t, 2.0) + 0.001 * cx.N[i]["label"])
            if kind == "session":
                rep = cx.idx["browser-512"]
            if kind == "container":
                rep = cx.idx["worker-134"]
            if kind == "kernel":
                rep = cx.idx["irq/142-nvme0q3"]
            y = 280 + j * 79
            self.braille(lay, rep, t, 0.50, 1224, y + 22, 56, 2, tuple(np.clip(np.array(S.KIND[kind]) * 1.1, 0, 255)))
            self.cards.append((kind, rep, y))
        lay.onto(canvas)
        img = K.downsample(canvas, self.w, self.h)
        self.text(img, t, sel)
        return img

    def braille(self, lay, i, t, span, x0, y0, chars, rows, colour, cw=6.2):
        """Membrane potential as a braille sparkline: each character cell is 2 x 4 dots."""
        cx, k = self.cx, self.k
        s1, s0 = cx.sidx(t), cx.sidx(t - span)
        v = cx.V[s0:s1, i]
        cols = chars * 2
        bins = np.array_split(v, cols)
        levels = rows * 4
        ch = 15.0

        def q(x):
            return int(np.clip(round((x + 78) / 108 * (levels - 1)), 0, levels - 1))
        for cidx, b in enumerate(bins):
            lo, hi = q(b.min()), q(b.max())
            for lv in range(lo, hi + 1):
                dx = (cidx // 2) * cw + (cidx % 2) * cw * 0.45 + 1.6
                rr = levels - 1 - lv
                dy = (rr // 4) * ch + (rr % 4) * ch * 0.25 + 2.0
                lay.circle(((x0 + dx) * k, (y0 + dy) * k), 1.05 * k, colour)

    def text(self, img, t, sel):
        cx, s = self.cx, self.s
        d = ImageDraw.Draw(img)

        def F(px, bold=False):
            return S.font(max(8, round(px * s)), S.FONT_MONO_BOLD if bold else S.FONT_MONO)

        def T(x, y, text, px=13, fill=S.TEXT, anchor="la", bold=False, shadow=True):
            f = F(px, bold)
            if shadow:
                d.text((x * s + 1, y * s + 1), text, font=f, fill=(0, 0, 0), anchor=anchor)
            d.text((x * s, y * s), text, font=f, fill=fill, anchor=anchor)
            return d.textbbox((x * s, y * s), text, font=f, anchor=anchor)

        # Layer labels.
        for name, kind, y0, y1 in LAYERS:
            ym = (y0 + y1) / 2
            if kind:
                T(24, ym - 11, name, 17, S.TEXT, bold=True)
                tn = TYPE_NAME[kind][0]
                T(24, ym + 10, f"{kind if kind != 'container' else 'containers'}  {tn}", 12,
                  tuple(min(255, int(c * 1.1)) for c in S.KIND[kind]))
            else:
                T(24, ym - 6, name, 17, S.DIM, bold=True)
                T(56, ym - 4, "molecular", 12, S.DIM)
        T(24, (WM[0] + WM[1]) / 2 - 6, "white matter", 12, S.DIM)
        # Neuron labels: placed under the soma, skipping any that would collide.
        placed = []
        for i, n in enumerate(cx.N):
            if not n["label"]:
                continue
            x, y = n["pos"]
            txt = n["name"]
            for (ox, oy, an) in ((0, 20 if n["name"] == sel else 13, "ma"), (0, -30, "md"), (14, -4, "lm"), (-14, -4, "rm")):
                f = F(11)
                bb = d.textbbox(((x + ox) * s, (y + oy) * s), txt, font=f, anchor=an)
                bb = (bb[0] - 3, bb[1] - 2, bb[2] + 3, bb[3] + 2)
                if bb[0] < SLICE_X[0] * s - 4 or bb[2] > (SLICE_X[1] + 6) * s:
                    continue
                if any(not (bb[2] < q[0] or bb[0] > q[2] or bb[3] < q[1] or bb[1] > q[3]) for q in placed):
                    continue
                placed.append(bb)
                hz = cx.rate(i, t, 2.0)
                T(x + ox, y + oy, txt, 11, (226, 228, 236), an)
                break
        # Inspector.
        si = cx.idx[sel]
        n = cx.N[si]
        hz = cx.rate(si, t, 2.0)
        tn = TYPE_NAME[n["kind"]]
        T(1224, 36, f"{n['name']}  {tn[1]}  {hz:.0f} Hz", 14, (240, 226, 170), bold=True)
        Ival = current(n["kind"], cpu_at(n, t), cx.cal)
        T(1224, 58, f"layer II/III session  {n['threads']} threads", 12, S.DIM)
        T(1224, 76, f"CPU {cpu_at(n, t) * 100:.0f}%   I = {Ival:.1f}   a b c d = 0.02 0.2 -50 2", 12, S.DIM)
        T(1574, 95, "+30 mV", 10, S.DIM, "ra", shadow=False)
        T(1574, 150, "-65", 10, S.DIM, "ra", shadow=False)
        T(1224, 162, "v(t), last 300 ms", 11, S.DIM)
        T(1224, 182, f"synapses in {sum(1 for a in cx.axons if si in (a['a'], a['b']))}"
                     f"   delay ~ axon length", 11, S.DIM)
        T(1224, 202, "spike pulses travel at 1.25 px/ms", 11, S.DIM)
        # Firing-type cards.
        T(1224, 254, "FIRING TYPES  Izhikevich 2003, dt 0.5 ms", 12, S.TEXT, bold=True)
        for kind, rep, y in self.cards:
            tn = TYPE_NAME[kind]
            col = tuple(min(255, int(c * 1.15)) for c in S.KIND[kind])
            hz = cx.rate(rep, t, 2.0)
            T(1224, y + 4, f"{tn[0]} {tn[1]}", 12, col, bold=True)
            T(1574, y + 4, f"{cx.N[rep]['name']} {hz:.0f} Hz", 11, S.DIM, "ra")
        # Raster labels.
        rx0, ry0, rx1, ry1 = RASTER
        T(rx0, ry0 - 18, f"RASTER  rows = neurons by layer ({len(self.raster_order())} that fire, of {cx.n}), dots = spikes, last 10 s", 12, S.TEXT)
        T(rx1, ry0 - 18, "-10 s ... now", 11, S.DIM, "ra")
        bounds = self.raster_bounds()
        for (name, kind, *_), y0, y1 in zip(LAYERS[1:], bounds[:-1], bounds[1:]):
            T(rx0 - 12, (y0 + y1) / 2, name, 11, tuple(S.KIND[kind]), "rm")
        T(rx1 + 2, ry1 + 4, "", 10)
        # Status.
        alln = [i for i in range(cx.n)]
        busy = [i for i in range(cx.n) if cx.rate(i, t, 2.0) > 0.4]
        web = [cx.idx[x] for x in WEB]
        chi_all = cx.chi(busy, t)
        chi_web = cx.chi(web, t)
        firing = sum(1 for i in alln if cx.rate(i, t, 1.0) > 0)
        sps = sum(cx.rate(i, t, 1.0) for i in alln)
        lines = [
            f"ISOTOP / CORTEX / DEMO   {cx.n} neurons | {firing} firing | {sps:,.0f} spikes/s | "
            f"synchrony chi {chi_all:.2f} (busy) | web stack chi {chi_web:.2f} | {len(cx.axons)} axons",
            "soma = process, layer by kind | dendrites = threads | axon = socket link, delay ~ length, "
            "weight ~ log traffic | input current ~ CPU",
            "FS kernel  RS system  CH session  IB containers | flash = spike | click a soma to inspect "
            "its membrane potential",
        ]
        S.status(img, lines, size=max(9, round(13 * s)))


def pack(frames, name, fps):
    """Saves the frames, then picks the largest GIF encoding that fits in 3.5 MB."""
    import gifpack
    S.save_frames(frames, name, fps=fps)
    for width, colors in ((720, 96), (720, 64), (640, 96), (640, 64), (560, 64), (480, 64)):
        path, size = gifpack.pack(name, fps=fps, width=width, colors=colors)
        print(f"  {width}px {colors}c: {size / 1e6:.2f} MB")
        if size <= 3.5e6:
            return path, size


def main(which):
    cx = Cortex()
    print('calibration (rheobase, I40):', cx.cal)
    cx.run()
    for nm in ("compiler-90", "nginx-128", "worker-134", "postgres-41", "browser-512", "journald-310",
               "irq/142-nvme0q3", "systemd-1"):
        i = cx.idx[nm]
        print(f"{nm:18s} {cx.rate(i, 2.0, 2.0):5.1f} Hz (t=2)  {cx.rate(i, 3.6, 1.0):5.1f} Hz (burst)")
    print("idle silent:", all(cx.spikes[i].size == 0 for i, n in enumerate(cx.N) if n["cpu"] == 0))
    if which in ("still", "both"):
        r = Renderer(cx, 1600, 900)
        img = r.frame(3.42)
        print(S.save_still(img, "cortex"))
    if which in ("anim", "both"):
        r = Renderer(cx, 960, 540)
        fps = 12
        frames = [r.frame(i / fps) for i in range(fps * 8)]
        print(pack(frames, "cortex", fps))
        K.contact_sheet([frames[i] for i in (10, 38, 44, 90)], os.path.join(S.OUT, "cortex-sheet.png"), width=1600)


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "both")
