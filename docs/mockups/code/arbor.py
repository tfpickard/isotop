"""isotop view mockup: arbor, the process tree as a great tree at dusk.

Trunk = PID 1, branch cross-sectional area = subtree memory (Leonardo's rule holds by
construction), children on golden-angle phyllotaxis slots in start order, leaves = threads
coloured by CPU, blossoms on new processes, kthreadd's kernel threads as the root system.
Run: python3 -I arbor.py [still|anim|both] [--frames a,b,c,d]
"""
import math
import os
import sys

PROTO = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, PROTO)

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

import isostyle as S

GOLDEN = math.radians(137.507764)


# ----------------------------------------------------------------------------- process tree
class Proc:
    def __init__(self, name, mem, threads=1, cpu=0.0, kind="system", born=None, children=None):
        self.name, self.mem, self.threads, self.cpu, self.kind = name, mem, threads, cpu, kind
        self.born = born              # seconds before "now" the process started (None = long ago)
        self.children = children or []
        self.parent = None

    def subtree(self):
        return self.mem + sum(c.subtree() for c in self.children)

    def walk(self):
        yield self
        for c in self.children:
            yield from c.walk()


def P(*a, **k):
    return Proc(*a, **k)


def build_tree():
    sys_ = dict(kind="system")
    ses = dict(kind="session")
    con = dict(kind="container")
    renderers = [P(f"renderer-{530 + i}", m, 22, 0.02, **ses) for i, m in enumerate([260, 220, 180, 180, 150, 120, 90])]
    browser = P("browser-512", 320, 36, 0.03, **ses, children=renderers + [
        P("gpu-process-520", 210, 14, 0.05, **ses), P("utility-524", 60, 8, 0.01, **ses),
        P("network-521", 50, 10, 0.01, **ses)])
    rustc = [P(f"rustc-{900 + i}", m, 16, 0.05, **ses, born=b)
             for i, (m, b) in enumerate([(240, 40), (210, 30), (180, 22), (160, 9), (140, 3), (120, 1.5)])]
    compiler = P("compiler-90", 70, 4, 0.02, **ses, children=rustc)
    shell = P("shell-170", 8, 1, 0.0, **ses, children=[compiler])
    user = P("systemd --user", 14, 1, 0.0, **ses, children=[
        P("pipewire-95", 30, 4, 0.03, **ses), P("gnome-shell-1104", 340, 38, 0.04, **ses), shell,
        P("language-server-71", 410, 40, 0.06, **ses), browser,
        P("terminal-1311", 90, 12, 0.01, **ses)])
    docker = P("dockerd", 92, 24, 0.01, **con, children=[
        P("containerd", 50, 14, 0.01, **con, children=[
            P("shim-ab12", 10, 10, 0.0, **con, children=[P("redis-207", 420, 6, 0.04, **con)]),
            P("shim-cd34", 10, 10, 0.0, **con, children=[P("worker-134", 300, 12, 0.25, **con)])])])
    pg = P("postgres-41", 180, 1, 0.01, **sys_, children=[
        P(n, m, 1, 0.01, **sys_) for n, m in [("checkpointer", 30), ("bgwriter", 22), ("walwriter", 18),
                                              ("autovacuum", 26), ("pg-backend-77", 48), ("pg-backend-78", 44)]])
    nginx = P("nginx-128", 12, 1, 0.0, **sys_, children=[P(f"nginx-w{i}", 18, 1, 0.02, **sys_) for i in range(4)])
    sshd = P("sshd", 8, 1, 0.0, **sys_, children=[P("sshd-session-801", 7, 1, 0.0, **sys_, children=[
        P("bash-802", 5, 1, 0.0, **sys_)]), P("sshd-session-950", 7, 1, 0.0, **sys_)])
    root = P("systemd-1", 14, 1, 0.0, **sys_, children=[
        P("journald-310", 48, 2, 0.01, **sys_), P("udevd-330", 12, 1, 0.0, **sys_),
        P("dbus-402", 6, 1, 0.0, **sys_), P("NetworkManager-415", 22, 5, 0.0, **sys_),
        P("polkitd-433", 9, 3, 0.0, **sys_), P("udisksd-440", 14, 5, 0.0, **sys_),
        P("cron-430", 3, 1, 0.0, **sys_), P("ModemManager-451", 11, 3, 0.0, **sys_),
        sshd, P("snapd-470", 44, 18, 0.01, **sys_), pg, P("accounts-daemon-482", 8, 3, 0.0, **sys_),
        P("fwupd-505", 62, 5, 0.0, **sys_), nginx, P("thermald-512", 6, 2, 0.01, **sys_),
        P("upowerd-530", 9, 3, 0.0, **sys_), docker, P("colord-560", 12, 3, 0.0, **sys_),
        P("gdm-610", 24, 4, 0.0, **sys_, children=[user]), P("packagekitd-700", 52, 3, 0.0, **sys_)])
    for p in root.walk():
        for c in p.children:
            c.parent = p
    # kernel threads: kthreadd's children, grouped by family for the root system
    fam = [("kworker", 34), ("ksoftirqd", 16), ("migration", 16), ("cpuhp", 16), ("rcu", 10), ("irq", 22),
           ("jbd2", 4), ("kswapd", 1), ("idle_inject", 16), ("watchdog", 16), ("kcompactd", 1), ("nvme", 9)]
    kroot = P("kthreadd", 0, 1, kind="kernel", children=[
        P(f, 0, n, kind="kernel", children=[P(f"{f}/{i}", 0, 1, kind="kernel") for i in range(n)])
        for f, n in fam])
    return root, kroot


# ----------------------------------------------------------------------------- geometry
def rot(v, axis, ang):
    axis = axis / np.linalg.norm(axis)
    return (v * math.cos(ang) + np.cross(axis, v) * math.sin(ang) + axis * np.dot(axis, v) * (1 - math.cos(ang)))


def perp_basis(d):
    a = np.array([0.0, 0.0, 1.0]) if abs(d[2]) < 0.9 else np.array([1.0, 0.0, 0.0])
    e1 = np.cross(d, a)
    e1 /= np.linalg.norm(e1)
    e2 = np.cross(d, e1)
    return e1, e2


K_RADIUS = 0.0036          # world radius per sqrt(MiB)


class Seg:
    __slots__ = ("p0", "p1", "r0", "r1", "proc", "depth", "kind", "root")

    def __init__(self, p0, p1, r0, r1, proc, depth, root=False):
        self.p0, self.p1, self.r0, self.r1, self.proc, self.depth, self.root = p0, p1, r0, r1, proc, depth, root


class Grower:
    def __init__(self, t, wind, rng_seed=11):
        self.t, self.wind = t, wind
        self.segs = []
        self.tips = {}          # proc -> (tip position, direction, length, radius)
        self.rng = np.random.default_rng(rng_seed)

    def grow(self, proc, base, d, length, depth=0, az0=0.0, mem_fn=None, root=False):
        mem_fn = mem_fn or (lambda p: p.subtree())
        rng = self.rng
        M = mem_fn(proc)
        kids = proc.children
        n = len(kids)
        # sway: a damped oscillator per branch; frequency falls with length, rises with thickness
        r_base = K_RADIUS * math.sqrt(max(M, 1.0)) if not root else 0.012 * math.sqrt(max(M, 1.0))
        phase = rng.uniform(0, 2 * math.pi)
        if self.wind > 0 and not root and depth > 0:
            freq = 0.9 * math.sqrt(r_base / 0.03) / math.sqrt(length)
            amp = self.wind * 0.07 * min(2.5, length / (r_base * 18 + 0.05))
            sway = np.array([1.0, 0.3, 0.0]) * amp * math.sin(2 * math.pi * freq * self.t * 0.35 + phase)
            d = d + sway
            d /= np.linalg.norm(d)
        # attachment fractions along this branch, start order from base to tip
        if n:
            lo = 0.5 if depth == 0 else 0.3
            fr = [lo + (1.0 - lo) * (i + 0.6) / (n + 0.2) for i in range(n)]
        else:
            fr = []
        steps = sorted(set([0.0, 1.0] + fr + [0.5]))
        # remaining area after each attachment (Leonardo's rule)
        own = proc.mem if not root else 1.0
        pts = []
        p = base.copy()
        cur = d.copy()
        bend_up = 0.10 if not root else -0.05
        e1, e2 = perp_basis(cur)
        wob = rng.normal(0, 0.12 if depth > 0 else 0.015, 2)
        prev_f = 0.0
        attach = {}
        for f in steps[1:]:
            seg_len = (f - prev_f) * length
            mid_dir = cur + np.array([0, 0, bend_up]) * seg_len + (e1 * wob[0] + e2 * wob[1]) * seg_len * 0.6
            mid_dir /= np.linalg.norm(mid_dir)
            q = p + mid_dir * seg_len
            pts.append((p, q, prev_f, f, mid_dir))
            p, cur = q, mid_dir
            prev_f = f
        for i, f in enumerate(fr):
            attach[i] = f
        # radii: before attachment i, area = own + sum of children not yet branched off
        def area_at(f):
            a = own + sum(mem_fn(kids[i]) for i in range(n) if attach[i] >= f - 1e-9)
            return a

        for (a, b, f0, f1, ddir) in pts:
            A0 = area_at(f0 + 1e-6)
            A1 = area_at(f1 - 1e-6)
            if root:
                r0, r1 = 0.012 * math.sqrt(max(A0, 0.4)), 0.012 * math.sqrt(max(A1, 0.4))
            else:
                r0, r1 = K_RADIUS * math.sqrt(max(A0, 1.0)), K_RADIUS * math.sqrt(max(A1, 1.0))
            self.segs.append(Seg(a, b, r0, r1, proc, depth, root))
        tip = pts[-1][1]
        self.tips[proc] = (tip, pts[-1][4], length, K_RADIUS * math.sqrt(max(proc.mem, 1.0)), pts)
        # children
        for i, c in enumerate(kids):
            f = attach[i]
            # find the point on the polyline
            for (a, b, f0, f1, ddir) in pts:
                if f0 - 1e-9 <= f <= f1 + 1e-9:
                    u = (f - f0) / max(f1 - f0, 1e-9)
                    base_c = a + (b - a) * u
                    pdir = ddir
                    break
            e1, e2 = perp_basis(pdir)
            az = az0 + (i + 1) * GOLDEN
            Mc = mem_fn(c)
            frac = Mc / max(M, 1e-6)
            if root:
                elev = math.radians(62 + 14 * (1 - frac)) if depth == 0 else math.radians(35 + 25 * rng.uniform())
            else:
                # heavy children stay near the parent's axis, light ones splay out
                elev = math.radians((34 + 40 * (1 - frac) ** 1.5) if depth == 0 else (36 + 38 * (1 - frac) ** 1.2))
            side = e1 * math.cos(az) + e2 * math.sin(az)
            cd = pdir * math.cos(elev) + side * math.sin(elev)
            if not root:
                cd = cd + np.array([0, 0, 0.06])
            else:
                cd = cd + np.array([0, 0, -0.15 if depth == 0 else -0.3])
            cd /= np.linalg.norm(cd)
            if root:
                Lc = (1.25 if depth == 0 else length * 0.42) * rng.uniform(0.8, 1.15)
            else:
                # grow towards the crown envelope: twigs reach it, inner limbs stop part way
                reach = envelope_reach(base_c, cd)
                lam = (0.5 + 0.42 * min(1.0, c.threads / 10)) if not c.children else 0.36 + 0.3 * frac ** 0.3
                Lc = max(0.22, lam * reach * rng.uniform(0.92, 1.05))
            self.grow(c, base_c, cd, Lc, depth + 1, az, mem_fn, root)


CROWN_C = np.array([0.0, 0.0, 2.7])
CROWN_R = np.array([3.7, 3.7, 1.95])


def envelope_reach(p, d):
    """Distance from p along d to the crown ellipsoid."""
    q = (p - CROWN_C) / CROWN_R
    v = d / CROWN_R
    a = v @ v
    b = 2 * q @ v
    c = q @ q - 1
    disc = b * b - 4 * a * c
    if disc <= 0:
        return 0.3
    t = (-b + math.sqrt(disc)) / (2 * a)
    return max(t, 0.3)


def spread_crown(gr, z0=1.3, z1=3.4, gain=0.85):
    """Widen the crown into a dome: horizontal stretch growing with height (skeleton only, so
    radii and therefore Leonardo's rule are untouched)."""
    def f(p):
        t = min(max((p[2] - z0) / (z1 - z0), 0.0), 1.0)
        t = t * t * (3 - 2 * t)
        q = p.copy()
        q[0] *= 1 + gain * t
        q[1] *= 1 + gain * t
        q[2] = p[2] - 0.18 * t * (p[0] ** 2 + p[1] ** 2) ** 0.5
        return q
    for s_ in gr.segs:
        s_.p0, s_.p1 = f(s_.p0), f(s_.p1)
    for p, (tip, d, L, r, pts) in list(gr.tips.items()):
        gr.tips[p] = (f(tip), d, L, r, [(f(a), f(b), f0, f1, dd) for (a, b, f0, f1, dd) in pts])


# ----------------------------------------------------------------------------- camera
class Cam:
    def __init__(self, W, H, ss, az_deg, el_deg=24.0, scale=106.0, cx=0.5, cy=0.73):
        self.th = math.radians(az_deg)
        self.ph = math.radians(el_deg)
        k = W / 1600
        self.S = scale * k * ss
        self.cx, self.cy = W * ss * cx, H * ss * cy
        self.ss = ss

    def proj(self, p):
        x, y, z = p[..., 0], p[..., 1], p[..., 2]
        c, s = math.cos(self.th), math.sin(self.th)
        u = x * c - y * s
        w = x * s + y * c
        X = self.cx + u * self.S
        Y = self.cy - (z * math.cos(self.ph) - w * math.sin(self.ph)) * self.S
        depth = w * math.cos(self.ph) + z * math.sin(self.ph)
        return X, Y, depth


def pr(cam, p):
    X, Y, D = cam.proj(np.asarray(p, np.float64)[None])
    return float(X[0]), float(Y[0]), float(D[0])


# ----------------------------------------------------------------------------- colours
BARK = np.array((70, 54, 50), np.float32)
BARK_LIT = np.array((168, 120, 132), np.float32)
ROOT = np.array((112, 124, 156), np.float32)


def leaf_colour(cpu):
    """green idle -> yellow -> red busy"""
    stops = [(0.0, (74, 150, 72)), (0.08, (104, 170, 70)), (0.3, (214, 196, 60)), (0.6, (236, 120, 44)),
             (1.0, (220, 52, 44))]
    for (a, ca), (b, cb) in zip(stops, stops[1:]):
        if cpu <= b:
            f = (cpu - a) / (b - a)
            return tuple(ca[i] + (cb[i] - ca[i]) * f for i in range(3))
    return stops[-1][1]


def mix(a, b, f):
    return tuple(int(a[i] + (b[i] - a[i]) * f) for i in range(3))


# ----------------------------------------------------------------------------- scene
def leaf_positions(gr, rng, procs):
    """One leaf per thread, scattered round the distal part of its process's twig."""
    leaves = []
    for p in procs:
        if p not in gr.tips:
            continue
        tip, d, L, r, pts = gr.tips[p]
        n = p.threads
        e1, e2 = perp_basis(d)
        for i in range(n):
            # along the last 45% of the branch and around the tip
            f = 1.0 - abs(rng.normal(0, 0.22)) if p.children == [] else rng.uniform(0.55, 1.0)
            f = min(max(f, 0.35), 1.0)
            seg = pts[min(int(f * len(pts)), len(pts) - 1)]
            a, b, f0, f1, dd = seg
            pos = a + (b - a) * rng.uniform(0, 1)
            spread = 0.12 + 0.14 * min(1.0, n / 20) + 0.04 * L
            off = (e1 * rng.normal(0, 1) + e2 * rng.normal(0, 1)) * spread + np.array([0, 0, abs(rng.normal(0, 0.1))])
            leaves.append(dict(pos=pos + off, proc=p, ang=rng.uniform(0, math.pi), size=rng.uniform(0.8, 1.2),
                               shade=rng.uniform(0.75, 1.15), tcpu=rng.dirichlet([0.6] * 1)[0], idx=i,
                               jit=rng.uniform(0, 2 * math.pi)))
    return leaves


def thread_cpu(p, i, t, anim):
    base = p.cpu
    if p.name.startswith("rustc") or p.name == "compiler-90":
        if anim:
            # the compile ramps up: codegen threads go from idle to flat out
            ramp = np.clip((t - 1.5 - (i % 7) * 0.35) / 3.0, 0, 1)
            return 0.02 + 0.93 * ramp * (0.75 + 0.25 * math.sin(i * 2.3))
        return 0.55 + 0.4 * math.sin(i * 1.7) ** 2
    if p.name == "worker-134":
        return 0.35 + 0.3 * math.sin(i * 1.3) ** 2
    if p.name.startswith("renderer-53") and p.name.endswith("0"):
        return 0.22
    return float(np.clip(base * (0.4 + 1.6 * abs(math.sin(i * 12.9898) * 43758.5453 % 1)), 0, 1))


def sky_layer(W, H):
    c = S.sky(W, H, glow=(166, 70, 140), base=(10, 9, 24), centre=(0.2, 0.32))
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    # dusk: a warm band low on the horizon behind the tree
    hor = np.exp(-(((yy - H * 0.66) / (H * 0.2)) ** 2)) * (0.55 + 0.45 * np.exp(-(((xx - W * 0.62) / (W * 0.5)) ** 2)))
    c += hor[..., None] * np.array((90, 40, 36), np.float32)
    rng = np.random.default_rng(4)
    for _ in range(int(W * H / 14000)):
        x, y = rng.uniform(0, W), rng.uniform(0, H * 0.45)
        S.glow(c, x, y, 0.5 * W / 1600 + 0.6, (200, 204, 236), strength=rng.uniform(0.15, 0.6))
    return c


def slab_polys(cam, half=2.35, depth=1.9):
    """The ground: a square slab, top face and the side faces that face the camera."""
    corners = np.array([(-half, -half), (half, -half), (half, half), (-half, half)])
    top = [pr(cam, (x, y, 0.0)) for x, y in corners]
    bot = [pr(cam, (x, y, -depth)) for x, y in corners]
    sides = []
    for i in range(4):
        j = (i + 1) % 4
        mid = (corners[i] + corners[j]) / 2
        # outward normal of this side; it faces the camera if it points towards +w
        nrm = mid / np.linalg.norm(mid)
        c, s = math.cos(cam.th), math.sin(cam.th)
        w = nrm[0] * s + nrm[1] * c
        if w > 0:
            sides.append(([top[i][:2], top[j][:2], bot[j][:2], bot[i][:2]], w))
    return [p[:2] for p in top], sides


def draw_branch(d, cam, seg, alpha=255, root=False, glow=None):
    x0, y0, _ = pr(cam, seg.p0)
    x1, y1, _ = pr(cam, seg.p1)
    r0, r1 = seg.r0 * cam.S, seg.r1 * cam.S
    dx, dy = x1 - x0, y1 - y0
    ln = math.hypot(dx, dy) + 1e-9
    nx, ny = -dy / ln, dx / ln
    if root:
        base = ROOT * (0.75 + 0.25 * min(1, seg.r0 * 30))
        lit = ROOT * 1.5
    else:
        base, lit = BARK, BARK_LIT
    body = [(x0 + nx * r0, y0 + ny * r0), (x1 + nx * r1, y1 + ny * r1), (x1 - nx * r1, y1 - ny * r1),
            (x0 - nx * r0, y0 - ny * r0)]
    col = tuple(int(v) for v in base) + (alpha,)
    d.polygon(body, fill=col)
    d.ellipse([x0 - r0, y0 - r0, x0 + r0, y0 + r0], fill=col)
    # rim light from the glowing sky on the left
    sgn = 1 if nx < 0 else -1
    a0, b0 = 0.95, 0.35
    rim = [(x0 + sgn * nx * r0 * a0, y0 + sgn * ny * r0 * a0), (x1 + sgn * nx * r1 * a0, y1 + sgn * ny * r1 * a0),
           (x1 + sgn * nx * r1 * b0, y1 + sgn * ny * r1 * b0), (x0 + sgn * nx * r0 * b0, y0 + sgn * ny * r0 * b0)]
    if r0 > 0.8 * cam.ss:
        d.polygon(rim, fill=tuple(int(v) for v in (base * 0.45 + lit * 0.55)) + (alpha,))
    if r0 > 3 * cam.ss and not root:
        # bark furrows
        for k in (-0.2, 0.25):
            d.line([(x0 + nx * r0 * k, y0 + ny * r0 * k), (x1 + nx * r1 * k, y1 + ny * r1 * k)],
                   fill=tuple(int(v * 0.7) for v in base) + (alpha,), width=max(1, int(cam.ss)))


def leaf_poly(x, y, ang, L, W):
    ca, sa = math.cos(ang), math.sin(ang)
    pts = [(L, 0), (L * 0.35, W), (-L * 0.6, W * 0.7), (-L, 0), (-L * 0.6, -W * 0.7), (L * 0.35, -W)]
    return [(x + u * ca - v * sa, y + u * sa + v * ca) for u, v in pts]


def blossom(d, x, y, r, open_f, rot0):
    if open_f <= 0.02:
        return
    rr = r * open_f
    for k in range(5):
        a = rot0 + k * 2 * math.pi / 5
        px, py = x + math.cos(a) * rr * 0.62, y + math.sin(a) * rr * 0.62
        d.ellipse([px - rr * 0.52, py - rr * 0.52, px + rr * 0.52, py + rr * 0.52], fill=(250, 196, 216, 240))
        d.ellipse([px - rr * 0.3, py - rr * 0.34, px + rr * 0.18, py + rr * 0.1], fill=(255, 236, 244, 230))
    d.ellipse([x - rr * 0.24, y - rr * 0.24, x + rr * 0.24, y + rr * 0.24], fill=(250, 214, 96, 255))


def gib(mib):
    return f"{mib / 1024:.1f} GiB" if mib >= 1024 else f"{mib:.0f} MiB"


def render(W, H, t, anim, az, cache, wind=1.0):
    k = W / 1600
    ss = 2
    lines = status_lines(short=W < 1200)
    sz = max(11, W // 115)
    band = int(sz * 1.45) * len(lines) + 10
    scene_h = H - band
    SW, SH = W * ss, scene_h * ss
    if ("sky", W) not in cache:
        cache[("sky", W)] = S.to_image(sky_layer(SW, SH))
    img = cache[("sky", W)].copy()
    cam = Cam(W, scene_h, ss, az, scale=106.0 * (0.93 if anim else 1.0), cy=0.74 if anim else 0.73)
    root, kroot = cache["trees"]
    gr = Grower(t, wind if anim else 0.6)
    gr.grow(root, np.array([0.0, 0.0, 0.0]), np.array([0.0, 0.0, 1.0]), 1.75, 0, 0.3)
    if "--bbox" in sys.argv:
        pts = np.array([s_.p1 for s_ in gr.segs])
        print("bbox", pts.min(0), pts.max(0))
    groot = Grower(t, 0.0, rng_seed=5)
    groot.grow(kroot, np.array([0.0, 0.0, -0.05]), np.array([0.0, 0.0, -1.0]), 0.7, 0, 0.0,
               mem_fn=lambda p: float(sum(1 for _ in p.walk())), root=True)
    rng = np.random.default_rng(21)
    procs = list(root.walk())
    leaves = leaf_positions(gr, rng, procs)
    d = ImageDraw.Draw(img, "RGBA")
    # --- below ground: roots, then translucent soil over them
    top, sides = slab_polys(cam)
    for poly, w in sides:
        d.polygon(poly, fill=(26, 20, 22, 255))
    rsegs = sorted(groot.segs, key=lambda s: pr(cam, (s.p0 + s.p1) / 2)[2])
    glow_layer = Image.new("L", img.size, 0)
    gd = ImageDraw.Draw(glow_layer)
    for s in rsegs:
        draw_branch(d, cam, s, 255, root=True)
        x0, y0, _ = pr(cam, s.p0)
        x1, y1, _ = pr(cam, s.p1)
        gd.line([(x0, y0), (x1, y1)], fill=120, width=max(2, int(s.r0 * cam.S * 2.5)))
    # root tips glow faintly (kernel threads)
    for p in kroot.walk():
        if p.children == [] and p in groot.tips:
            x, y, _ = pr(cam, groot.tips[p][0])
            gd.ellipse([x - 3 * ss, y - 3 * ss, x + 3 * ss, y + 3 * ss], fill=200)
    glow_layer = glow_layer.filter(ImageFilter.GaussianBlur(5 * ss))
    gl = np.asarray(glow_layer, np.float32)[..., None] / 255.0
    arr = np.asarray(img.convert("RGB"), np.float32) + gl * np.array((70, 90, 150), np.float32)
    img = Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8))
    d = ImageDraw.Draw(img, "RGBA")
    # soil: translucent faces with strata
    for poly, w in sides:
        d.polygon(poly, fill=(62, 44, 40, 105))
        (a, b, c_, e) = poly
        for f in (0.22, 0.5, 0.78):
            p0 = (a[0] + (e[0] - a[0]) * f, a[1] + (e[1] - a[1]) * f)
            p1 = (b[0] + (c_[0] - b[0]) * f, b[1] + (c_[1] - b[1]) * f)
            d.line([p0, p1], fill=(110, 80, 64, 70), width=max(1, ss))
        d.line([a, b], fill=(120, 150, 100, 200), width=max(1, ss * 2))
    d.polygon(top, fill=(38, 60, 44, 150))
    # grass tufts on the top face
    rg = np.random.default_rng(9)
    for _ in range(260):
        gx, gy = rg.uniform(-2.25, 2.25, 2)
        x, y, _ = pr(cam, (gx, gy, 0))
        h = rg.uniform(2, 6) * ss * k
        d.line([(x, y), (x + rg.uniform(-1, 1) * ss, y - h)], fill=(80, 130, 86, 150), width=max(1, ss))
    # fallen leaves on the grass (threads that exited earlier)
    rl = np.random.default_rng(17)
    for _ in range(26):
        rr_ = rl.uniform(0.6, 2.2)
        aa = rl.uniform(0, 2 * math.pi)
        x, y, _ = pr(cam, (rr_ * math.cos(aa), rr_ * math.sin(aa), 0))
        col = leaf_colour(rl.uniform(0.3, 0.9))
        Lf = 9 * ss * k
        d.polygon(leaf_poly(x, y, rl.uniform(0, math.pi), Lf, Lf * 0.28), fill=tuple(int(c * 0.6) for c in col) + (230,))
    # shadow of the crown on the ground
    sh = Image.new("L", img.size, 0)
    sdraw = ImageDraw.Draw(sh)
    for lf in leaves[::3]:
        x, y, _ = pr(cam, (lf["pos"][0] * 0.8 + 0.6, lf["pos"][1] * 0.8 + 0.4, 0.0))
        sdraw.ellipse([x - 9 * ss, y - 5 * ss, x + 9 * ss, y + 5 * ss], fill=50)
    tm = Image.new("L", img.size, 0)
    ImageDraw.Draw(tm).polygon(top, fill=255)
    sh = sh.filter(ImageFilter.GaussianBlur(8 * ss))
    sh = Image.fromarray((np.asarray(sh, np.float32) * np.asarray(tm, np.float32) / 255).astype(np.uint8))
    img = Image.composite(Image.new("RGB", img.size, (10, 14, 16)), img, sh)
    d = ImageDraw.Draw(img, "RGBA")
    # --- above ground: branches, leaves, blossoms sorted back to front
    items = []
    for s in gr.segs:
        items.append((pr(cam, (s.p0 + s.p1) / 2)[2] - 0.02, 0, s))
    # leaves
    fall = []
    for lf in leaves:
        p = lf["proc"]
        cpu = thread_cpu(p, lf["idx"], t, anim)
        pos = lf["pos"].copy()
        if anim:
            pos = pos + np.array([1.0, 0.3, 0.0]) * 0.035 * wind * math.sin(t * 2.1 + lf["jit"]) * max(0.0, pos[2] - 2.0) * 0.3
        items.append((pr(cam, pos)[2], 1, (lf, cpu, pos)))
    # falling leaves (threads that exited)
    nfall = 9
    rf = np.random.default_rng(33)
    for i in range(nfall):
        src = leaves[int(rf.integers(len(leaves)))]
        t0 = rf.uniform(-6, 9) if anim else rf.uniform(0.5, 6)
        tt = (t - t0) % 9.0 if anim else t0
        z0 = src["pos"][2]
        z = z0 - tt * 0.55
        if z < 0.05:
            continue
        pos = src["pos"] + np.array([0.35 * tt + 0.25 * math.sin(tt * 2.2 + i), 0.2 * math.cos(tt * 1.7 + i), 0])
        pos[2] = z
        items.append((pr(cam, pos)[2] + 0.5, 2, (pos, tt * 3.1 + i, rf.uniform(0, 0.25))))
    # blossoms on new processes
    for p in procs:
        if p.born is None or p not in gr.tips:
            continue
        tip, dd, L, r, pts = gr.tips[p]
        age = p.born
        if anim:
            age = p.born + (t - 0.0) - 4.5 if p.name in ("rustc-904", "rustc-905") else p.born + t
        open_f = float(np.clip((5.5 - age) / 0.8, 0, 1)) * float(np.clip(age / 0.7 + 0.15, 0, 1)) if age > -0.5 else 0
        if p.name in ("rustc-904", "rustc-905") and anim:
            open_f = float(np.clip((t - 2.5 - (0.6 if p.name == "rustc-905" else 0)) / 1.3, 0, 1))
        if open_f <= 0:
            continue
        e1, e2 = perp_basis(dd)
        rb = np.random.default_rng(hash(p.name) % 1000)
        for j in range(6):
            off = (e1 * rb.normal(0, 1) + e2 * rb.normal(0, 1)) * 0.2 + dd * rb.uniform(-0.25, 0.1)
            items.append((pr(cam, tip + off)[2] + 0.01, 3, (tip + off, open_f * rb.uniform(0.8, 1.15), rb.uniform(0, 6))))
    items.sort(key=lambda it: it[0])
    halo = []
    for depth, typ, obj in items:
        if typ == 0:
            draw_branch(d, cam, obj)
        elif typ == 1:
            lf, cpu, pos = obj
            x, y, dep = pr(cam, pos)
            col = leaf_colour(cpu)
            # back leaves darker, lit edge from the sky glow
            # front leaves brighter; the magenta sky lights the upper left of the crown
            rel = np.clip((cam.cx - x) / (3.5 * cam.S) + (pr(cam, (0, 0, 2.7))[1] - y) / (3.0 * cam.S), -1, 1)
            f = lf["shade"] * (0.58 + 0.34 * np.clip((dep + 3.5) / 7, 0, 1) + 0.2 * rel)
            col = tuple(int(min(255, c * f)) for c in col)
            L = 13.5 * ss * k * lf["size"]
            d.polygon(leaf_poly(x, y, lf["ang"], L, L * 0.5), fill=col + (255,))
            d.line([(x - math.cos(lf["ang"]) * L * 0.7, y - math.sin(lf["ang"]) * L * 0.7),
                    (x + math.cos(lf["ang"]) * L * 0.7, y + math.sin(lf["ang"]) * L * 0.7)],
                   fill=tuple(int(c * 0.7) for c in col) + (200,), width=1)
            if cpu > 0.6:
                halo.append((x, y, cpu))
        elif typ == 2:
            pos, spin, cpu = obj
            x, y, _ = pr(cam, pos)
            L = 7.0 * ss * k
            sq = abs(math.cos(spin))
            col = leaf_colour(0.4 + cpu)
            d.polygon(leaf_poly(x, y, spin * 0.5, L, L * 0.5 * (0.3 + 0.7 * sq)), fill=tuple(int(c * 0.9) for c in col) + (235,))
        else:
            pos, open_f, rot0 = obj
            x, y, _ = pr(cam, pos)
            halo.append((x, y, -1))
            blossom(d, x, y, 7.5 * ss * k, open_f, rot0)
    # busy leaves and blossoms glow a little (additive)
    gl = Image.new("RGB", img.size, (0, 0, 0))
    gd = ImageDraw.Draw(gl)
    for x, y, c in halo:
        rr = (10 if c < 0 else 7) * ss * k
        col = (120, 60, 90) if c < 0 else (int(110 * c), int(40 * c), 10)
        gd.ellipse([x - rr, y - rr, x + rr, y + rr], fill=col)
    gl = gl.filter(ImageFilter.GaussianBlur(9 * ss * k))
    img = Image.fromarray(np.clip(np.asarray(img, np.int16) + np.asarray(gl, np.int16), 0, 255).astype(np.uint8))
    small = img.resize((W, scene_h), Image.LANCZOS)
    full = Image.new("RGB", (W, H), S.PANEL)
    full.paste(small, (0, 0))
    ov = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    od = ImageDraw.Draw(ov)
    draw_labels(od, cam, gr, groot, root, kroot, W, scene_h, k, ss, anim, cache if anim else None)
    full.paste(ov, (0, 0), ov)
    S.status(full, lines, size=sz)
    return full


def find(root, name):
    for p in root.walk():
        if p.name == name:
            return p
    return None


def draw_labels(od, cam, gr, groot, root, kroot, W, scene_h, k, ss, anim=False, cache=None):
    lab = max(10, round(13 * k))
    f = S.font(lab)
    reqs = []

    def mid(name, frac, g):
        p = find(root, name) or find(kroot, name)
        pts = g.tips[p][4]
        a, b = pts[0][0], pts[-1][1]
        return a + (b - a) * frac, p

    def info(p):
        return f"{gib(p.subtree())}  {sum(q.threads for q in p.walk())} threads"

    names = (("systemd --user", 0.5), ("browser-512", 0.6), ("dockerd", 0.55), ("sshd", 0.7),
             ("compiler-90", 0.9), ("postgres-41", 0.6))
    if anim:
        names = (("systemd --user", 0.5), ("browser-512", 0.6), ("dockerd", 0.55), ("compiler-90", 0.9))
    for name, frac in names:
        pos, p = mid(name, frac, gr)
        text, sub = name, info(p)
        if name == "dockerd":
            text = "docker"
        if name == "browser-512":
            text = f"browser-512 ({gib(p.subtree())})"
            sub = f"{sum(q.threads for q in p.walk())} leaves = threads"
        if name == "compiler-90":
            sub = "compiling: leaves redden"
        reqs.append((pos, text, sub, tuple(int(c) for c in S.KIND[p.kind])))
    tp = gr.tips[root][4]
    reqs.append((tp[0][0] + (tp[-1][1] - tp[0][0]) * 0.3, "systemd-1", f"trunk = PID 1, {gib(root.subtree())}",
                 tuple(int(c) for c in S.KIND["system"])))
    kp = groot.tips[kroot][4]
    n_k = sum(1 for q in kroot.walk()) - 1
    reqs.append((kp[-1][1] + np.array([0, 0, -0.5]), "kthreadd", f"roots = {n_k} kernel threads",
                 tuple(int(c) for c in S.KIND["kernel"])))
    # project, split by side, then stack each side top to bottom without overlaps
    items = []
    for pos, text, sub, col in reqs:
        x, y, _ = pr(cam, pos)
        items.append([x / ss, y / ss, text, sub, col])
    cxs = cam.cx / ss
    gap = lab * 3.0
    for side0 in (-1, 1):
        group = sorted([it for it in items if (it[0] < cxs) == (side0 < 0)], key=lambda it: it[1])
        last = -1e9
        for it in group:
            x, y, text, sub, col = it
            side = side0
            ty = max(y - 18 * k, last + gap, lab * 1.2)
            last = ty
            if cache is not None:
                # keep each label at the offset it got on the first frame so labels do not jump
                key = ("lab", text)
                if key not in cache:
                    cache[key] = (ty - y, side)
                ty = y + cache[key][0]
                side = cache[key][1]
            tw = max(f.getlength(text), S.font(max(9, lab - 2)).getlength(sub))
            tx = x + side * 70 * k
            tx = min(max(tx, tw + 12 if side < 0 else 8), W - tw - 12 if side > 0 else W)
            bx0 = tx - (tw if side < 0 else 0) - 6 * k
            bx1 = tx + (0 if side < 0 else tw) + 6 * k
            od.line([(x, y), (tx - side * 3, ty + lab * 0.3)], fill=col + (200,), width=1)
            od.ellipse([x - 2.5, y - 2.5, x + 2.5, y + 2.5], fill=col + (255,))
            od.rounded_rectangle([bx0, ty - lab * 0.8, bx1, ty + lab * 1.95], radius=4, fill=(12, 12, 22, 175))
            anchor = "rm" if side < 0 else "lm"
            S.label(od, tx, ty, text, size=lab, fill=col, anchor=anchor)
            S.label(od, tx, ty + lab * 1.2, sub, size=max(9, lab - 2), fill=(170, 174, 192), anchor=anchor)


def status_lines(short=False):
    if short:
        return ["ISOTOP / ARBOR / DEMO   192 processes | 941 threads | load 6.1 | RAM 20.0 / 32.0 GiB",
                "trunk = PID 1 | branch area = subtree memory | leaves = threads, green idle to red busy",
                "blossoms = new processes | falling leaves = exited threads | roots = kernel threads"]
    return ["ISOTOP / ARBOR / DEMO   192 processes | 941 threads | load 6.1 | RAM 20.0 GiB / 32.0 GiB | "
            "wind = CPU pressure 30%",
            "Trunk = PID 1 | branch cross-section = subtree memory (Leonardo's rule) | children on golden-angle "
            "slots in start order | leaves = threads: green idle, yellow, red busy",
            "blossoms = processes started in the last 5 s | falling leaves = exited threads | roots = kthreadd's "
            "kernel threads | drag to orbit | Tab next view | q quit"]


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "still"
    cache = {"trees": build_tree()}
    if mode in ("still", "both"):
        img = render(1600, 900, 0.0, False, 38.0, cache)
        print("still", S.save_still(img, "arbor"))
    if mode in ("anim", "both"):
        fps, dur = 12, 9.0
        n = int(fps * dur)
        picks = list(range(n))
        if "--frames" in sys.argv:
            picks = [int(v) for v in sys.argv[sys.argv.index("--frames") + 1].split(",")]
        out = []
        for fi in picks:
            t = fi / fps
            az = 30.0 + 22.0 * t / dur
            out.append(render(960, 540, t, True, az, cache))
        if "--frames" in sys.argv:
            W, H = 960, 540
            sheet = Image.new("RGB", (W * 2, H * 2))
            for j, f in enumerate(out[:4]):
                sheet.paste(f, ((j % 2) * W, (j // 2) * H))
            sheet.save(S.OUT + "/arbor-sheet.png")
            print("sheet")
        else:
            print(S.save_frames(out, "arbor", fps=fps))


if __name__ == "__main__":
    main()
