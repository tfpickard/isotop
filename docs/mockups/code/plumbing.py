"""Mockup of isotop's plumbing view: inter-process plumbing as a Victorian boiler room.

Every process is a riveted iron tank (size ~ memory, band = kind) with a pressure gauge. Pipes
and FIFOs are copper runs routed with A* on a coarse grid (Manhattan, elbows, stable); glass
sections show water dashes moving at the writer's rate. Unix sockets are brass pneumatic tubes
overhead with capsules (one per K unread bytes). Loopback TCP is a green garden hose. A process
with several writable pipes feeds a manifold labelled "split unknown". A writer blocked in
pipe_write has its gauge in the red and its pipe bulging. A pipe whose reader exited bursts.

usage: python3 -I plumbing.py [still|anim|both]
"""
import heapq
import math
import os
import sys

PROTO = "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto"
sys.path.insert(0, PROTO)

import cv2  # noqa: E402
import numpy as np  # noqa: E402
from PIL import Image, ImageDraw, ImageFont  # noqa: E402

import isostyle as S  # noqa: E402
import mockkit as K  # noqa: E402

N = 20                      # floor is N x N grid cells; world units are cells
LIGHT = K.LIGHT
VIEW = K.VIEW
HALF = (LIGHT + VIEW) / np.linalg.norm(LIGHT + VIEW)

COPPER = np.array([196, 104, 56], np.float32)
BRASS = np.array([206, 166, 78], np.float32)
IRON = np.array([40, 66, 54], np.float32)
HOSE = np.array([62, 156, 66], np.float32)
GLASS = np.array([190, 214, 222], np.float32)
WATER = np.array([40, 120, 190], np.float32)
WATER_HI = np.array([150, 220, 255], np.float32)

PIPE_R = 0.17
PIPE_Z = 0.42
TUBE_R = 0.20
TUBE_Z = 3.5
WALL_H = 3.0

T_EXIT = 3.6      # gzip-219 (reader of redis's pipe) exits
T_BURST = 4.4     # the pipe bursts

# name: kind, x, y, memory MiB
TANKS = {
    "browser-512": ("session", 3.2, 4.0, 2150),
    "pipewire-95": ("session", 3.0, 10.4, 38),
    "compiler-90": ("session", 6.4, 14.2, 640),
    "language-server-71": ("session", 2.6, 15.8, 910),
    "shell-170": ("session", 5.6, 18.4, 8),
    "systemd-1": ("system", 10.6, 2.2, 14),
    "journald-310": ("system", 14.4, 2.4, 62),
    "nginx-128": ("system", 18.0, 3.0, 44),
    "postgres-41": ("container", 11.4, 9.0, 1230),
    "worker-134": ("container", 15.4, 8.0, 150),
    "redis-207": ("container", 18.4, 10.4, 310),
    "gzip-219": ("session", 18.6, 16.8, 3),
    "cat-301": ("session", 8.8, 15.4, 2),
    "grep-302": ("session", 11.0, 16.0, 3),
    "sort-303": ("session", 13.4, 16.6, 96),
    "uniq-304": ("session", 15.8, 17.2, 2),
}
PIPELINE = ["cat-301", "grep-302", "sort-303", "uniq-304"]

ZONES = [  # cgroup floor markings: name, x0, y0, x1, y1
    ("user.slice", 0.3, 0.3, 8.4, 19.7),
    ("system.slice", 8.6, 0.3, 19.7, 5.6),
    ("docker", 8.6, 5.8, 19.7, 13.0),
    ("session-3.scope", 8.6, 13.2, 19.7, 19.7),
]

MANIFOLD = (7.0, 9.6)   # compiler-90 has three writable pipes: split unknown


def tank_r(mem):
    return 0.30 + 0.085 * math.log2(max(mem, 2) / 2)


def tank_h(mem):
    return 1.6 * tank_r(mem) + 0.55


def fmt_mem(mem):
    return f"{mem / 1024:.1f} GiB" if mem >= 1024 else f"{mem} MiB"


# Pipes: (src, dst, kind, state, rate B/s). Endpoint "MANIFOLD" is the split box.
PIPES = [
    ("browser-512", "pipewire-95", "pipe", "blocked", 0),
    ("compiler-90", "MANIFOLD", "pipe", "flow", 3.1e6),
    ("MANIFOLD", "language-server-71", "pipe", "split", 1.03e6),
    ("MANIFOLD", "journald-310", "pipe", "split", 1.03e6),
    ("MANIFOLD", "postgres-41", "pipe", "split", 1.03e6),
    ("cat-301", "grep-302", "pipe", "flow", 48e6),
    ("grep-302", "sort-303", "pipe", "flow", 2.2e6),
    ("sort-303", "uniq-304", "pipe", "dry", 0),
    ("redis-207", "gzip-219", "pipe", "burst", 9e6),
]
SOCKETS = [  # unix stream sockets, overhead pneumatic tubes: src, dst, queued bytes
    ("systemd-1", "journald-310", 12288),
    ("nginx-128", "worker-134", 28672),
]
KCAP = 4096
HOSE_ENDS = ("worker-134", "postgres-41")


# ---------------------------------------------------------------------------------------------
# Routing: A* on a half-cell grid around the tanks, with a turn penalty.

RES = 2  # nodes per cell


def obstacles(exclude=()):
    n = N * RES + 1
    occ = np.zeros((n, n), bool)
    ii, jj = np.mgrid[0:n, 0:n] / RES
    for name, (_, x, y, mem) in TANKS.items():
        if name in exclude:
            continue
        r = tank_r(mem) + 0.55
        occ |= (ii - x) ** 2 + (jj - y) ** 2 <= r * r
    mx, my = MANIFOLD
    occ |= (np.abs(ii - mx) <= 0.9) & (np.abs(jj - my) <= 0.9)
    occ[0, :] = occ[-1, :] = occ[:, 0] = occ[:, -1] = True
    return occ


def astar(occ, used, a, b):
    n = occ.shape[0]
    dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)]
    start = (a, -1)
    g = {start: 0.0}
    prev = {}
    pq = [(0.0, a, -1)]
    while pq:
        f, node, d = heapq.heappop(pq)
        if node == b:
            path = [node]
            key = (node, d)
            while key in prev:
                key = prev[key]
                path.append(key[0])
            return path[::-1]
        cost0 = g[(node, d)]
        for k, (dx, dy) in enumerate(dirs):
            nx, ny = node[0] + dx, node[1] + dy
            if not (0 <= nx < n and 0 <= ny < n):
                continue
            if occ[nx, ny] and (nx, ny) != b:
                continue
            c = cost0 + 1.0 + (2.5 if d not in (-1, k) else 0.0) + used[nx, ny] * 7.0
            key = ((nx, ny), k)
            if c < g.get(key, 1e18):
                g[key] = c
                prev[key] = (node, d)
                h = abs(nx - b[0]) + abs(ny - b[1])
                heapq.heappush(pq, (c + h, (nx, ny), k))
    return None


def simplify(path):
    pts = [path[0]]
    for i in range(1, len(path) - 1):
        a, b, c = path[i - 1], path[i], path[i + 1]
        if (b[0] - a[0], b[1] - a[1]) != (c[0] - b[0], c[1] - b[1]):
            pts.append(b)
    pts.append(path[-1])
    return pts


def port(name, toward):
    """Grid node just outside a tank (or the manifold) on the axis facing `toward`."""
    if name == "MANIFOLD":
        x, y = MANIFOLD
        r = 0.5
    else:
        _, x, y, mem = TANKS[name]
        r = tank_r(mem)
    dx, dy = toward[0] - x, toward[1] - y
    if abs(dx) >= abs(dy):
        d = (np.sign(dx) or 1, 0)
    else:
        d = (0, np.sign(dy) or 1)
    px = round((x + d[0] * (r + 0.6)) * RES)
    py = round((y + d[1] * (r + 0.6)) * RES)
    surf = (x + d[0] * r * 0.92, y + d[1] * r * 0.92)
    return (px, py), surf, d


def centre(name):
    if name == "MANIFOLD":
        return MANIFOLD
    return TANKS[name][1], TANKS[name][2]


def route_all():
    occ = obstacles()
    used = np.zeros_like(occ, np.float32)
    runs = []
    for src, dst, kind, state, rate in PIPES:
        (pa, sa, _), (pb, sb, _) = port(src, centre(dst)), port(dst, centre(src))
        o = occ.copy()
        for (px_, py_) in (pa, pb):
            o[max(px_ - 1, 0):px_ + 2, max(py_ - 1, 0):py_ + 2] = False
            o[px_, py_] = False
        path = astar(o, used, pa, pb)
        if path is None:
            raise RuntimeError(f"no route {src}->{dst}")
        for p in path:
            used[p] += 1
            for q in ((p[0] + 1, p[1]), (p[0] - 1, p[1]), (p[0], p[1] + 1), (p[0], p[1] - 1)):
                if 0 <= q[0] < used.shape[0] and 0 <= q[1] < used.shape[1]:
                    used[q] += 0.35
        pts = [(sa[0], sa[1])] + [(p[0] / RES, p[1] / RES) for p in simplify(path)] + [(sb[0], sb[1])]
        # Snap the stub ends so the first and last legs stay axis aligned.
        pts[0] = (pts[0][0], pts[1][1]) if abs(pts[1][0] - pts[0][0]) > abs(pts[1][1] - pts[0][1]) else (pts[1][0], pts[0][1])
        pts[-1] = (pts[-1][0], pts[-2][1]) if abs(pts[-2][0] - pts[-1][0]) > abs(pts[-2][1] - pts[-1][1]) else (pts[-2][0], pts[-1][1])
        runs.append(dict(src=src, dst=dst, kind=kind, state=state, rate=rate, pts=pts))
    return runs


# ---------------------------------------------------------------------------------------------
# Shaders: everything paints straight onto a float canvas, back to front.


def paint(canvas, x0, y0, col, cov):
    h, w = cov.shape
    region = canvas[y0:y0 + h, x0:x0 + w]
    region[:] = region * (1 - cov[..., None]) + col * cov[..., None]


def bbox(canvas, xs, ys, pad):
    H, W = canvas.shape[:2]
    x0 = max(int(math.floor(min(xs) - pad)), 0)
    x1 = min(int(math.ceil(max(xs) + pad)) + 1, W)
    y0 = max(int(math.floor(min(ys) - pad)), 0)
    y1 = min(int(math.ceil(max(ys) + pad)) + 1, H)
    return x0, y0, x1, y1


def metal(nrm, base, amb=0.20, kd=0.85, ks=0.9, rough=36, tint=0.6):
    diffuse = np.clip(nrm @ LIGHT, 0, 1)
    spec = np.clip(nrm @ HALF, 0, 1) ** rough
    up = np.clip(nrm[..., 2], 0, 1)
    spec_col = base * tint + 255 * (1 - tint)
    col = base * (amb + kd * diffuse + 0.12 * up)[..., None] + spec_col * (ks * spec)[..., None]
    return col


def tube(canvas, geo, P0, P1, r0, r1, base, mode="metal", opacity=1.0, s0=0.0, emis=None, **mk):
    """Shaded cylinder (or cone) between world points, analytic per pixel. mode 'glass' gives a
    fresnel-weighted transparent shell. emis(s_world, q) -> extra RGB for water dashes."""
    P0, P1 = np.asarray(P0, float), np.asarray(P1, float)
    d = P1 - P0
    Ld = float(np.linalg.norm(d))
    if Ld < 1e-6:
        return
    dh = d / Ld
    up = np.array([0, 0, 1.0]) if abs(dh[2]) < 0.9 else np.array([1, -1, 0]) / math.sqrt(2)
    p1 = np.cross(dh, up)
    p1 /= np.linalg.norm(p1)
    p2 = np.cross(p1, dh)
    S0 = np.array(geo.p(*P0), float)
    S1 = np.array(geo.p(*P1), float)
    sd = S1 - S0
    sl = float(np.linalg.norm(sd))
    if sl < 1e-3:
        return
    sdh = sd / sl
    nn = np.array([-sdh[1], sdh[0]])
    e1 = np.array(geo.dirv(*p1))
    e2 = np.array(geo.dirv(*p2))
    a1, a2 = nn @ e1, nn @ e2
    hwu = math.hypot(a1, a2)
    t0 = math.atan2(a2, a1)
    rmax = max(r0, r1) * hwu
    x0, y0, x1, y1 = bbox(canvas, [S0[0], S1[0]], [S0[1], S1[1]], rmax + 2)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    vx, vy = xx - S0[0], yy - S0[1]
    sp = vx * sdh[0] + vy * sdh[1]
    sq = vx * nn[0] + vy * nn[1]
    frac = np.clip(sp / sl, 0, 1)
    r = r0 + (r1 - r0) * frac
    hw = np.maximum(r * hwu, 1e-3)
    cov = np.clip(hw - np.abs(sq) + 0.5, 0, 1) * np.clip(sp + 0.5, 0, 1) * np.clip(sl - sp + 0.5, 0, 1)
    if cov.max() <= 0:
        return
    q = np.clip(sq / hw, -1, 1)
    ac = np.arccos(q)
    na = np.cos(t0 + ac)[..., None] * p1 + np.sin(t0 + ac)[..., None] * p2
    nb = np.cos(t0 - ac)[..., None] * p1 + np.sin(t0 - ac)[..., None] * p2
    nrm = np.where(((na @ VIEW) >= (nb @ VIEW))[..., None], na, nb)
    if mode == "glass":
        fres = 1 - np.clip(nrm @ VIEW, 0, 1)
        a = (0.10 + 0.75 * fres ** 2.2) * opacity
        spec = np.clip(nrm @ HALF, 0, 1) ** 60
        col = base * (0.35 + 0.5 * np.clip(nrm @ LIGHT, 0, 1))[..., None] + 255 * spec[..., None] * 1.2
        a = np.clip(a + spec * 0.9, 0, 1)
        paint(canvas, x0, y0, col, cov * a)
        return
    col = metal(nrm, base, **mk)
    if emis is not None:
        col = col + emis(s0 + frac * Ld, q)
    paint(canvas, x0, y0, col, cov * opacity)


def ball(canvas, geo, P, r, base, opacity=1.0, **mk):
    """Shaded sphere at a world point (elbow fittings, capsule ends)."""
    cx, cy = geo.p(*P)
    rad = r * geo.s * 1.0
    x0, y0, x1, y1 = bbox(canvas, [cx], [cy], rad + 2)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    dx, dy = (xx - cx) / rad, (yy - cy) / rad
    d2 = dx * dx + dy * dy
    dz = np.sqrt(np.clip(1 - d2, 0, 1))
    # Screen basis to world: right = (1,-1,0)/sqrt2, up = vertical-ish, out = VIEW.
    right = np.array([1, -1, 0]) / math.sqrt(2)
    upv = np.cross(VIEW, right)
    nrm = dx[..., None] * right - dy[..., None] * upv + dz[..., None] * VIEW
    col = metal(nrm, base, **mk)
    cov = np.clip((1 - np.sqrt(d2)) * rad + 0.5, 0, 1) * opacity
    paint(canvas, x0, y0, col, cov)


def tank(canvas, geo, x, y, r, h, kind_col, rng, gauge=0.4, cap=BRASS, fade=1.0, drain=0.0):
    """Riveted iron tank: shaded body, kind-coloured band, brass-rimmed domed cap, valve."""
    s = geo.s
    a = math.sqrt(2) * r * s
    b = a / 2
    cx, cy = geo.p(x, y, 0)
    top = h * s
    x0, y0, x1, y1 = bbox(canvas, [cx - a, cx + a], [cy - top - b - 0.6 * s, cy + b], 3)
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    du = xx - cx
    u = np.clip(du / a, -1, 1)
    half = b * np.sqrt(np.clip(1 - u * u, 0, 1))
    body_top = cy - top - half
    body_bot = cy + half
    cov_x = np.clip(a - np.abs(du) + 0.5, 0, 1)
    cov = cov_x * np.clip(yy - (cy - top) + 0.5, 0, 1) * np.clip(body_bot - yy + 0.5, 0, 1)
    psi = np.arccos(u)
    phi = psi - math.pi / 4
    nrm = np.dstack([np.cos(phi), np.sin(phi), np.zeros_like(phi)])
    z = (cy + b * np.sin(psi) - yy) / s
    base = np.broadcast_to(IRON, nrm.shape).copy()
    # Plate seams and a painted band in the kind colour.
    band = (z > h * 0.58) & (z < h * 0.74)
    base[band] = np.array(kind_col, np.float32) * 0.85
    seam = (np.abs(z - h * 0.3) < 0.035) | (np.abs(z - h * 0.58) < 0.03) | (np.abs(z - h * 0.74) < 0.03)
    base[seam] *= 0.55
    grime = 1 + 0.06 * rng.standard_normal(z.shape).astype(np.float32)
    ao = 0.55 + 0.45 * np.clip(z / 0.9, 0, 1)
    col = metal(nrm, base, amb=0.22, kd=0.80, ks=0.55, rough=22, tint=0.75) * (grime * ao)[..., None]
    if drain > 0:
        col *= (1 - 0.7 * drain)
    paint(canvas, x0, y0, col, cov * fade)
    # Rivets along the band edges.
    for zz in (h * 0.58, h * 0.74, h * 0.3):
        for k in range(14):
            ph = -math.pi / 4 + (k + 0.5) / 14 * math.pi * 1.0 - math.pi / 4 + math.pi / 4
            ph = math.radians(-40) + k / 13 * math.radians(170)
            P = (x + r * 1.01 * math.cos(ph), y + r * 1.01 * math.sin(ph), zz + (0.06 if zz != h * 0.3 else 0.0))
            if (math.cos(ph) + math.sin(ph)) < 0.15:
                continue
            ball(canvas, geo, P, 0.045 + 0.02 * min(r, 1.5), IRON * 1.4, opacity=fade, ks=0.6, rough=20)
    # Domed cap.
    cap_h = 0.22 * r + 0.1
    ex, ey = cx, cy - top
    q = np.sqrt((du / a) ** 2 + ((yy - ey) / b) ** 2)
    capm = np.clip((1 - q) * b + 0.5, 0, 1)
    # Dome normal: world offset from the cap centre.
    dv = yy - ey
    wx = (du / s + 2 * dv / s) / 2
    wy = (2 * dv / s - du / s) / 2
    k = 0.9 * cap_h / max(r, 1e-3)
    dn = np.dstack([wx / r * k, wy / r * k, np.ones_like(wx)])
    dn /= np.linalg.norm(dn, axis=2, keepdims=True)
    ccol = metal(dn, IRON * 1.15, amb=0.25, ks=0.5, rough=18)
    # Lift the dome slightly by drawing it with a raised highlight ring (brass rim).
    rim = np.clip(1 - np.abs(q - 0.94) / 0.06, 0, 1)
    rimcol = metal(dn * np.array([1, 1, 0.6]), cap, ks=1.0, rough=28)
    ccol = ccol * (1 - rim[..., None]) + rimcol * rim[..., None]
    paint(canvas, x0, y0, ccol, capm * fade)
    # Valve stem and wheel on top.
    vz = h + cap_h * 0.6
    tube(canvas, geo, (x, y, h), (x, y, vz + 0.18), 0.07, 0.07, cap, opacity=fade, ks=1.0)
    wx_, wy_ = geo.p(x, y, vz + 0.2)
    wa = 0.22 * s * math.sqrt(2)
    lay = K.Layer(x1 - x0, y1 - y0)
    lay.ellipse((wx_ - x0, wy_ - y0), (wa, wa / 2), 0, (150, 40, 34), thickness=max(2, 0.06 * s))
    lay.onto(canvas[y0:y1, x0:x1], fade)
    return dict(top=(cx, cy - top - b), gauge=gauge)


def gauge_draw(canvas, geo, x, y, r, h, value, red=False, t=0.0):
    """Brass pressure gauge on the front of a tank (a circle in a plane facing the viewer)."""
    s = geo.s
    cx, cy = geo.p(x + r * 0.71, y + r * 0.71, h * 0.42)
    gr = min(0.42 * r, 0.55) * s
    sx, sy = math.sqrt(2) * 0.72, 0.78   # foreshortening of a disc facing (1,1,0)
    lay = K.Layer(canvas.shape[1], canvas.shape[0])
    x0, y0, x1, y1 = bbox(canvas, [cx], [cy], gr * 1.6)
    sub = canvas[y0:y1, x0:x1]
    lay = K.Layer(x1 - x0, y1 - y0)
    c = (cx - x0, cy - y0)
    lay.ellipse(c, (gr * 1.18 * sx, gr * 1.18 * sy), 0, (120, 92, 40))
    lay.ellipse(c, (gr * 1.08 * sx, gr * 1.08 * sy), 0, (222, 186, 96))
    lay.ellipse(c, (gr * sx, gr * sy), 0, (226, 218, 196) if not red else (236, 214, 200))
    # Red zone from 80% to 100% of a 270 degree sweep.
    def ang(v):
        return math.radians(225 - 270 * v)
    ra = []
    for v in np.linspace(0.78, 1.0, 16):
        ra.append((c[0] + math.cos(ang(v)) * gr * 0.82 * sx, c[1] - math.sin(ang(v)) * gr * 0.82 * sy))
    lay.polyline(ra, (196, 40, 36), width=max(2, gr * 0.22))
    for v in np.linspace(0, 1, 9):
        a_ = ang(v)
        p0 = (c[0] + math.cos(a_) * gr * 0.62 * sx, c[1] - math.sin(a_) * gr * 0.62 * sy)
        p1 = (c[0] + math.cos(a_) * gr * 0.86 * sx, c[1] - math.sin(a_) * gr * 0.86 * sy)
        lay.line(p0, p1, (60, 50, 40), width=max(1, gr * 0.06))
    a_ = ang(value)
    tip = (c[0] + math.cos(a_) * gr * 0.80 * sx, c[1] - math.sin(a_) * gr * 0.80 * sy)
    lay.line(c, tip, (30, 24, 20) if not red else (150, 20, 20), width=max(2, gr * 0.11))
    lay.circle(c, gr * 0.13, (60, 46, 30))
    lay.onto(sub)
    # Glass glint.
    S.glow(canvas, cx - gr * 0.4, cy - gr * 0.45, gr * 0.22, (255, 255, 240), 0.35)
    if red:
        S.glow(canvas, cx, cy, gr * 1.4, (255, 60, 40), 0.30 + 0.12 * math.sin(t * 9))


# ---------------------------------------------------------------------------------------------
# Static scenery: floor tiles with cgroup paint, brick walls with arched windows.


def floor_texture(n, rng):
    u = (np.arange(n, dtype=np.float32) + 0.5) / n * N
    X, Y = np.meshgrid(u, u)
    tile = 1.0
    ix, iy = np.floor(X / tile), np.floor(Y / tile)
    h = (np.sin(ix * 12.9898 + iy * 78.233) * 43758.5453) % 1.0
    base = np.array([44, 46, 50], np.float32)
    tone = 0.82 + 0.3 * h
    noise = K.value_noise(n, 30, rng, ((1, 1.0), (3, 0.5))) * 0.06
    grain = 0.06 * rng.standard_normal((n, n)).astype(np.float32)
    tex = base * (tone + noise + grain)[..., None]
    fx, fy = X / tile - ix, Y / tile - iy
    grout = np.minimum(np.minimum(fx, 1 - fx), np.minimum(fy, 1 - fy))
    gm = np.clip(1 - grout / 0.035, 0, 1)
    tex = tex * (1 - 0.6 * gm[..., None]) + np.array([16, 16, 18], np.float32) * 0.6 * gm[..., None]
    # Bevel highlight on the tile edge facing the light.
    bev = np.clip(1 - fx / 0.06, 0, 1) * (fx < 0.06) + np.clip(1 - fy / 0.06, 0, 1) * (fy < 0.06)
    tex += 10 * bev[..., None]
    # Grime and oil stains.
    stains = np.clip(K.value_noise(n, 7, rng, ((1, 1.0), (2.2, 0.5))) - 0.6, 0, None)
    tex *= (1 - 0.35 * np.clip(stains, 0, 1))[..., None]
    # Painted cgroup boundaries and stencilled names (faded yellow safety paint).
    img = Image.new("L", (n, n), 0)
    dr = ImageDraw.Draw(img)
    k = n / N
    fnt = ImageFont.truetype(S.FONT_MONO_BOLD, int(0.42 * k))
    for name, x0, y0, x1, y1 in ZONES:
        dr.rectangle([x0 * k, y0 * k, x1 * k, y1 * k], outline=255, width=max(2, int(0.07 * k)))
        dr.text((x0 * k + 0.35 * k, y1 * k - 0.35 * k), name.upper(), font=fnt, fill=255, anchor="ld")
    paint_m = np.asarray(img, np.float32) / 255.0
    wear = np.clip(0.55 + 0.6 * K.value_noise(n, 40, rng), 0, 1)
    paint_m *= wear
    tex = tex * (1 - 0.55 * paint_m[..., None]) + np.array([170, 140, 60], np.float32) * 0.55 * paint_m[..., None]
    # Dim toward the rim; warm lamp pools.
    rim = np.minimum(np.minimum(X, N - X), np.minimum(Y, N - Y)) / N
    tex *= (0.70 + 0.30 * np.clip(rim / 0.12, 0, 1))[..., None]
    for lx, ly, rad, st in ((6, 8, 5.5, 0.55), (13, 6, 5.0, 0.45), (13, 16, 5.0, 0.5)):
        g = np.exp(-((X - lx) ** 2 + (Y - ly) ** 2) / (2 * rad * rad))
        tex += g[..., None] * np.array([90, 60, 26], np.float32) * st
    return tex


def wall_texture(length, height, k, rng, windows):
    """Brick wall texture (rows from the top down) with arched windows; length x height cells."""
    w, h = int(length * k), int(height * k)
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32) / k
    bh, bw = 0.22, 0.5
    row = np.floor(yy / bh)
    off = (row % 2) * bw / 2
    fx = ((xx + off) % bw) / bw
    fy = (yy % bh) / bh
    bid = np.floor((xx + off) / bw) + row * 97
    hsh = (np.sin(bid * 12.9898) * 43758.5453) % 1.0
    brick = np.array([88, 40, 30], np.float32) * (0.65 + 0.45 * hsh)[..., None]
    mortar = (np.minimum(fx, 1 - fx) < 0.045) | (np.minimum(fy, 1 - fy) < 0.08)
    tex = np.where(mortar[..., None], np.array([44, 38, 36], np.float32), brick)
    tex *= (1 + 0.08 * rng.standard_normal((h, w)).astype(np.float32))[..., None]
    # Soot darkening toward the top and a stone plinth at the base.
    tex *= (0.55 + 0.45 * (yy / height))[..., None]
    plinth = yy > height - 0.35
    tex[plinth] = np.array([70, 66, 60], np.float32) * (0.8 + 0.2 * hsh[plinth])[..., None]
    alpha = np.ones((h, w), np.float32)
    glow = np.zeros((h, w), np.float32)
    for wc in windows:
        ww, wh, wb = 1.1, 1.7, 0.75   # width, height of the straight part, sill height from base
        top = height - wb - wh
        inside_rect = (np.abs(xx - wc) < ww / 2) & (yy > top) & (yy < height - wb)
        inside_arch = ((xx - wc) ** 2 + (yy - top) ** 2 < (ww / 2) ** 2) & (yy <= top)
        win = inside_rect | inside_arch
        frame = (((np.abs(xx - wc) < ww / 2 + 0.09) & (yy > top) & (yy < height - wb + 0.09))
                 | (((xx - wc) ** 2 + (yy - top) ** 2 < (ww / 2 + 0.09) ** 2) & (yy <= top))) & ~win
        tex[frame] = np.array([120, 112, 100], np.float32) * 0.6
        # Mullions: glazing bars.
        bars = (np.abs(xx - wc) < 0.025) | (np.abs(((yy - top) % 0.42)) < 0.025)
        glass = np.array([30, 60, 86], np.float32) * (0.9 + 0.5 * ((height - wb - yy) / wh))[..., None]
        tex = np.where((win & ~bars)[..., None], glass, tex)
        tex = np.where((win & bars)[..., None], np.array([26, 26, 30], np.float32), tex)
        glow += win * 1.0
    # Coping stones at the top.
    cop = yy < 0.12
    tex[cop] = np.array([96, 92, 86], np.float32)
    return tex, glow


class Scene:
    def __init__(self, width, height, ss=2, tex_n=1600):
        self.W, self.H, self.ss = width, height, ss
        self.geo = K.Geo(width, height, ss, size=N, frac=0.80, top=0.205)
        g = self.geo
        rng = np.random.default_rng(7)
        self.runs = route_all()
        wss, hss = width * ss, height * ss
        canvas = S.sky(wss, hss, glow=(150, 60, 120))
        # Slab sides.
        lay = K.Layer(wss, hss)
        th = 0.55
        lay.poly([g.p(0, N), g.p(N, N), g.p(N, N, -th), g.p(0, N, -th)], (40, 38, 40))
        lay.poly([g.p(N, N), g.p(N, 0), g.p(N, 0, -th), g.p(N, N, -th)], (26, 25, 28))
        lay.onto(canvas)
        # Floor.
        tex = floor_texture(tex_n, rng)
        M = g.texture_matrix(tex_n)
        ground = cv2.warpAffine(tex, M, (wss, hss), flags=cv2.INTER_LINEAR, borderValue=0)
        mask = cv2.warpAffine(np.ones((tex_n, tex_n), np.float32), M, (wss, hss), flags=cv2.INTER_LINEAR)
        canvas = canvas * (1 - mask[..., None]) + ground * mask[..., None]
        self.floor_mask = mask
        # Walls on the two back edges (x = 0 and y = 0).
        k = 60
        for axis, wins in ((0, (4.0, 10.0, 16.0)), (1, (4.0, 10.0, 16.0))):
            wt, wg = wall_texture(N, WALL_H, k, rng, wins)
            h_, w_ = wt.shape[:2]
            if axis == 0:   # plane y = 0, runs along x
                src = np.float32([[0, 0], [w_, 0], [0, h_]])
                dst = np.float32([g.p(0, 0, WALL_H), g.p(N, 0, WALL_H), g.p(0, 0, 0)])
                shade = 0.62
            else:           # plane x = 0, runs along y
                src = np.float32([[0, 0], [w_, 0], [0, h_]])
                dst = np.float32([g.p(0, 0, WALL_H), g.p(0, N, WALL_H), g.p(0, 0, 0)])
                shade = 0.95
            A = cv2.getAffineTransform(src, dst)
            img = cv2.warpAffine(wt * shade, A, (wss, hss), flags=cv2.INTER_LINEAR)
            m = cv2.warpAffine(np.ones((h_, w_), np.float32), A, (wss, hss), flags=cv2.INTER_LINEAR)
            gl = cv2.warpAffine(wg, A, (wss, hss), flags=cv2.INTER_LINEAR)
            canvas = canvas * (1 - m[..., None]) + img * m[..., None]
            canvas += cv2.GaussianBlur(gl, (0, 0), 14 * ss)[..., None] * np.array([20, 50, 80], np.float32) * 0.9
        # Wall-top coping edge and corner pilaster.
        lay = K.Layer(wss, hss)
        lay.line(g.p(0, N, WALL_H), g.p(0, 0, WALL_H), (120, 112, 104), 2 * ss)
        lay.line(g.p(0, 0, WALL_H), g.p(N, 0, WALL_H), (90, 84, 78), 2 * ss)
        lay.line(g.p(0, 0, 0), g.p(0, 0, WALL_H), (30, 26, 26), 2 * ss)
        lay.onto(canvas)
        # Contact shadows of tanks and pipes on the floor.
        sh = np.zeros((hss, wss), np.float32)
        for name, (kind, x, y, mem) in TANKS.items():
            if name == "gzip-219":
                continue
            r = tank_r(mem)
            cx, cy = g.p(x + 0.25, y + 0.25)
            a = math.sqrt(2) * r * 1.25 * g.s
            cv2.ellipse(sh, (int(cx), int(cy)), (int(a), int(a / 2)), 0, 0, 360, 1.0, -1, cv2.LINE_AA)
        for run in self.runs:
            pts = [g.p(px + 0.12, py + 0.12) for px, py in run["pts"]]
            cv2.polylines(sh, [np.int32(pts).reshape(-1, 1, 2)], False, 0.8, int(PIPE_R * 3 * g.s), cv2.LINE_AA)
        sh = cv2.GaussianBlur(sh, (0, 0), 6 * ss)
        canvas *= (1 - 0.55 * np.clip(sh, 0, 1) * mask)[..., None]
        self.static = canvas
        self.rng = rng

    # -----------------------------------------------------------------------------------------

    def pipe_items(self, run, t):
        """Depth-sortable pieces of one copper run: plain copper, glass windows, collars, elbows."""
        g = self.geo
        pts3 = [(x, y, PIPE_Z) for x, y in run["pts"]]
        # Rise up from the tank wall at the pipe height (stubs are already at the tank surface).
        items = []
        segs = list(zip(pts3[:-1], pts3[1:]))
        lens = [math.dist(a, b) for a, b in segs]
        total = sum(lens)
        # Glass window: in the longest segment, centred, up to 2.2 cells long.
        gi = int(np.argmax(lens))
        state = run["state"]
        rate = run["rate"]
        speed = 0.0 if rate <= 0 else 0.6 + 0.45 * math.log10(max(rate, 1e3) / 1e3)
        burst = state == "burst" and t >= T_BURST
        acc = 0.0
        for i, ((a, b), L) in enumerate(zip(segs, lens)):
            a, b = np.array(a), np.array(b)
            dvec = (b - a) / max(L, 1e-9)
            cuts = [0.0, L]
            glass = None
            if i == gi and L > 1.6:
                gl = min(2.2, L - 0.8)
                if state == "blocked":
                    gl = min(2.6, L - 0.6)
                g0 = (L - gl) / 2
                glass = (g0, g0 + gl)
                cuts = [0.0, g0, g0 + gl, L]
            gap = None
            if state == "burst" and i == len(segs) - 1:
                # The joint nearest the vanished reader gives way.
                gp = max(0.6, L * 0.45)
                gap = (gp - 0.22, gp + 0.22)
            # Plain copper pieces, split into half cells for depth sorting.
            spans = []
            if glass:
                spans = [(0.0, glass[0]), (glass[1], L)]
            else:
                spans = [(0.0, L)]
            for (u0, u1) in spans:
                if gap and burst:
                    parts = []
                    for (v0, v1) in ((u0, min(u1, gap[0])), (max(u0, gap[1]), u1)):
                        if v1 > v0:
                            parts.append((v0, v1))
                else:
                    parts = [(u0, u1)]
                for v0, v1 in parts:
                    nseg = max(1, int(math.ceil((v1 - v0) / 0.5)))
                    for k in range(nseg):
                        w0 = v0 + (v1 - v0) * k / nseg
                        w1 = v0 + (v1 - v0) * (k + 1) / nseg
                        P0, P1 = a + dvec * w0, a + dvec * w1
                        key = (P0[0] + P1[0] + P0[1] + P1[1]) / 2
                        items.append((key, ("copper", P0, P1)))
            if glass:
                P0, P1 = a + dvec * glass[0], a + dvec * glass[1]
                key = (P0[0] + P1[0] + P0[1] + P1[1]) / 2
                items.append((key, ("glass", P0, P1, state, speed, acc + glass[0], t)))
                for P in (P0, P1):
                    items.append((P[0] + P[1] + 0.01, ("collar", P - dvec * 0.08, P + dvec * 0.08)))
            if i > 0:
                items.append((a[0] + a[1] + 0.02, ("elbow", a)))
            if gap and burst:
                gp = (gap[0] + gap[1]) / 2
                P = a + dvec * gp
                items.append((P[0] + P[1] + 0.05, ("burst", a + dvec * gap[0], a + dvec * gap[1], dvec, t)))
            acc += L
        # Brass flanges where the run meets each tank.
        a0, a1 = np.array(pts3[0]), np.array(pts3[1])
        d0 = (a1 - a0) / np.linalg.norm(a1 - a0)
        items.append((a0[0] + a0[1] + 0.03, ("collar", a0 + d0 * 0.05, a0 + d0 * 0.2)))
        b0, b1 = np.array(pts3[-1]), np.array(pts3[-2])
        d1 = (b1 - b0) / np.linalg.norm(b1 - b0)
        if not (state == "burst" and t >= T_EXIT):
            items.append((b0[0] + b0[1] + 0.03, ("collar", b0 + d1 * 0.05, b0 + d1 * 0.2)))
        return items

    def draw_item(self, canvas, item):
        g = self.geo
        kind = item[0]
        if kind == "copper":
            _, P0, P1 = item
            tube(canvas, g, P0, P1, PIPE_R, PIPE_R, COPPER, ks=1.0, rough=30, tint=0.7)
        elif kind == "collar":
            _, P0, P1 = item
            tube(canvas, g, P0, P1, PIPE_R * 1.38, PIPE_R * 1.38, BRASS, ks=1.1, rough=26)
        elif kind == "elbow":
            ball(canvas, g, item[1], PIPE_R * 1.25, COPPER * 0.95, ks=1.0, rough=26)
        elif kind == "glass":
            _, P0, P1, state, speed, s0, t = item
            self.glass_section(canvas, P0, P1, state, speed, s0, t)
        elif kind == "burst":
            _, P0, P1, dvec, t = item
            self.burst(canvas, P0, P1, dvec, t)
        elif kind == "tank":
            _, name, t = item
            self.draw_tank(canvas, name, t)
        elif kind == "manifold":
            self.manifold(canvas)
        elif kind == "fn":
            item[1](canvas)

    def glass_section(self, canvas, P0, P1, state, speed, s0, t):
        g = self.geo
        P0, P1 = np.asarray(P0), np.asarray(P1)
        if state == "blocked":
            # Bulging glass, packed with still water under pressure.
            n = 18
            pulse = 1 + 0.05 * math.sin(t * 7.0)
            for k in range(n):
                u0, u1 = k / n, (k + 1) / n
                bump = lambda u: 1 + 0.75 * pulse * math.sin(math.pi * u) ** 1.6
                A, B = P0 + (P1 - P0) * u0, P0 + (P1 - P0) * u1
                tube(canvas, g, A, B, PIPE_R * 0.85 * bump(u0), PIPE_R * 0.85 * bump(u1), WATER * 0.75,
                     ks=0.5, rough=18,
                     emis=lambda s, q: (np.clip(1 - np.abs(q), 0, 1) ** 2 * 50)[..., None] * np.array([0.6, 0.8, 1.0], np.float32))
            for k in range(n):
                u0, u1 = k / n, (k + 1) / n
                bump = lambda u: 1 + 0.75 * pulse * math.sin(math.pi * u) ** 1.6
                A, B = P0 + (P1 - P0) * u0, P0 + (P1 - P0) * u1
                tube(canvas, g, A, B, PIPE_R * bump(u0), PIPE_R * bump(u1), GLASS * np.array([1.1, 0.85, 0.8]),
                     mode="glass", opacity=1.0)
            return
        if state != "dry":
            def emis(s, q):
                ph = (s - t * speed * 2.2) % 0.55
                dash = np.clip(1 - np.abs(ph - 0.16) / 0.12, 0, 1)
                core = np.clip(1 - np.abs(q), 0, 1) ** 0.7
                return (dash * core)[..., None] * (WATER_HI - WATER) * 1.3
            tube(canvas, g, P0, P1, PIPE_R * 0.72, PIPE_R * 0.72, WATER, ks=0.6, rough=20, emis=emis, s0=s0)
        else:
            # Dry: a few drops left in the bottom of the glass.
            for k in range(3):
                P = P0 + (P1 - P0) * (0.25 + 0.25 * k)
                ball(canvas, g, (P[0], P[1], P[2] - PIPE_R * 0.6), 0.045, WATER_HI, ks=1.0)
        tube(canvas, g, P0, P1, PIPE_R, PIPE_R, GLASS, mode="glass")

    def burst(self, canvas, P0, P1, dvec, t):
        """Ragged copper ends and the dark bore showing at the break."""
        g = self.geo
        rng = np.random.default_rng(int(t * 1000) % 9973)
        for P, sgn in ((P0, 1), (P1, -1)):
            # Peeled petals of copper.
            for k in range(5):
                ang = k / 5 * 2 * math.pi + 0.4
                up = np.array([0, 0, 1.0])
                side = np.cross(dvec, up)
                rad = side * math.cos(ang) + up * math.sin(ang)
                A = np.asarray(P) + rad * PIPE_R * 0.9
                B = A + dvec * sgn * 0.12 + rad * 0.16
                tube(canvas, g, A, B, 0.045, 0.02, COPPER * 0.9, ks=0.9)

    def spray(self, canvas, run, t, glow_only=False):
        """Water spray from the burst: ballistic droplets, mist and a spreading puddle."""
        if t < T_BURST:
            return
        g = self.geo
        pts = run["pts"]
        a, b = np.array(pts[-2] + (PIPE_Z,)), np.array(pts[-1] + (PIPE_Z,))
        L = np.linalg.norm(b - a)
        dvec = (b - a) / L
        P = a + dvec * max(0.6, L * 0.45)
        age = t - T_BURST
        rng = np.random.default_rng(31)
        n = 380
        side = np.cross(dvec, [0, 0, 1.0])
        cx, cy = g.p(*P)
        lay_pts = []
        for i in range(n):
            # Each droplet repeats on its own phase (a steady jet).
            period = rng.uniform(0.45, 0.9)
            ph = (age + rng.uniform(0, period)) % period
            if age < ph:
                continue
            sgn = rng.choice([-1.0, 1.0])
            v = (side * sgn * rng.uniform(0.6, 2.4) + dvec * rng.normal(0, 0.8)
                 + np.array([0, 0, 1.0]) * rng.uniform(1.8, 4.2))
            pos = P + v * ph + np.array([0, 0, -4.9]) * ph * ph
            if pos[2] < 0:
                continue
            lay_pts.append((pos, rng.uniform(0.5, 1.0)))
        layer = np.zeros(canvas.shape[:2], np.float32)
        for pos, w in lay_pts:
            sx, sy = g.p(*pos)
            if 0 <= int(sx) < layer.shape[1] and 0 <= int(sy) < layer.shape[0]:
                cv2.circle(layer, (int(sx * 16), int(sy * 16)), int(16 * self.ss * (0.9 + 0.7 * w)), w, -1, cv2.LINE_AA, 4)
        canvas += layer[..., None] * np.array([150, 200, 230], np.float32) * 0.65
        canvas += cv2.GaussianBlur(layer, (0, 0), 7 * self.ss)[..., None] * np.array([90, 140, 190], np.float32) * 0.7
        S.glow(canvas, cx, cy - 1.0 * g.s, 1.6 * g.s, (130, 170, 200), 0.22)

    def puddle(self, canvas, run, t):
        if t < T_BURST:
            return
        g = self.geo
        pts = run["pts"]
        a, b = np.array(pts[-2]), np.array(pts[-1])
        L = np.linalg.norm(b - a)
        P = a + (b - a) / L * max(0.6, L * 0.45)
        grow = min(1.0, 0.35 + (t - T_BURST) / 4.0)
        rx = 1.7 * grow
        lay = K.Layer(canvas.shape[1], canvas.shape[0])
        pts_ = []
        rng = np.random.default_rng(4)
        wob = rng.uniform(0.85, 1.15, 12)
        for k in range(48):
            th = k / 48 * 2 * math.pi
            rr = rx * (1 + 0.12 * math.sin(3 * th + 1) + 0.08 * math.sin(5 * th))
            pts_.append(g.p(P[0] + rr * math.cos(th) * 1.2, P[1] + rr * math.sin(th) * 0.9, 0.02))
        lay.poly(pts_, (24, 46, 66))
        lay.onto(canvas, 0.75)
        cx, cy = g.p(P[0], P[1])
        for k in range(3):
            rr = ((t * 0.9 + k / 3) % 1.0) * rx * 0.9
            ring = K.Layer(canvas.shape[1], canvas.shape[0])
            ring.ellipse((cx, cy), (rr * math.sqrt(2) * g.s, rr * math.sqrt(2) * g.s / 2), 0,
                         (150, 200, 230), thickness=1.3 * self.ss)
            ring.onto(canvas, 0.35 * (1 - ((t * 0.9 + k / 3) % 1.0)))

    def draw_tank(self, canvas, name, t):
        g = self.geo
        kind, x, y, mem = TANKS[name]
        r, h = tank_r(mem), tank_h(mem)
        rng = np.random.default_rng(sum(map(ord, name)))
        fade, drain = 1.0, 0.0
        if name == "gzip-219":
            fade = float(np.clip(1 - (t - T_EXIT) / 0.7, 0, 1))
            if fade <= 0:
                # A faint outline where the reader used to stand.
                cx, cy = g.p(x, y)
                a = math.sqrt(2) * r * g.s
                lay = K.Layer(canvas.shape[1], canvas.shape[0])
                for k in range(0, 360, 30):
                    lay.ellipse((cx, cy), (a, a / 2), 0, (170, 150, 150), thickness=1.2 * self.ss, start=k, end=k + 16)
                lay.onto(canvas, 0.6)
                return
        tank(canvas, g, x, y, r, h, S.KIND[kind], rng, fade=fade, drain=drain)
        if fade > 0.5:
            value, red = self.gauge_value(name, t)
            gauge_draw(canvas, g, x, y, r, h, value, red, t)

    def gauge_value(self, name, t):
        """Gauge = fill of the tank's busiest outgoing pipe (0..1); red when blocked."""
        base = {
            "browser-512": 0.97, "compiler-90": 0.42, "cat-301": 0.55, "grep-302": 0.30,
            "sort-303": 0.06, "redis-207": 0.36, "nginx-128": 0.22, "systemd-1": 0.12,
            "journald-310": 0.18, "postgres-41": 0.33, "worker-134": 0.27, "language-server-71": 0.15,
            "shell-170": 0.04, "pipewire-95": 0.05, "uniq-304": 0.02, "gzip-219": 0.2,
        }[name]
        h = sum(map(ord, name))
        wob = 0.05 * math.sin(t * (1.3 + h % 5 * 0.4) + h) + 0.025 * math.sin(t * 5.1 + h * 0.3)
        if name == "browser-512":
            wob = 0.012 * math.sin(t * 23) + 0.01 * math.sin(t * 37)
        if name == "redis-207" and t >= T_BURST:
            base = 0.05 + 0.3 * math.exp(-(t - T_BURST) * 2)
        return float(np.clip(base + wob, 0, 1)), name == "browser-512"

    def manifold(self, canvas):
        g = self.geo
        mx, my = MANIFOLD
        tube(canvas, g, (mx, my - 0.75, PIPE_Z), (mx, my + 0.75, PIPE_Z), 0.34, 0.34, BRASS, ks=1.1, rough=24)
        tube(canvas, g, (mx - 0.75, my, PIPE_Z), (mx + 0.75, my, PIPE_Z), 0.30, 0.30, BRASS * 0.95, ks=1.1, rough=24)
        ball(canvas, g, (mx, my, PIPE_Z + 0.05), 0.42, BRASS, ks=1.2, rough=30)
        # Little stopcock wheel on top.
        tube(canvas, g, (mx, my, PIPE_Z + 0.3), (mx, my, PIPE_Z + 0.75), 0.06, 0.06, BRASS)
        cx, cy = g.p(mx, my, PIPE_Z + 0.78)
        lay = K.Layer(canvas.shape[1], canvas.shape[0])
        wa = 0.3 * g.s * math.sqrt(2)
        lay.ellipse((cx, cy), (wa, wa / 2), 0, (150, 40, 34), thickness=0.08 * g.s)
        lay.line((cx - wa, cy), (cx + wa, cy), (150, 40, 34), 0.05 * g.s)
        lay.onto(canvas)

    # Pneumatic tubes (unix sockets) -----------------------------------------------------------

    def socket_path(self, src, dst):
        _, xa, ya, ma = TANKS[src]
        _, xb, yb, mb = TANKS[dst]
        ra, rb = tank_r(ma), tank_r(mb)
        ha, hb = tank_h(ma), tank_h(mb)
        # Leave the top of each tank (through the cap), rise to TUBE_Z, Manhattan across.
        A0 = np.array([xa, ya, ha + 0.25])
        B0 = np.array([xb, yb, hb + 0.25])
        A1 = np.array([xa, ya, TUBE_Z])
        B1 = np.array([xb, yb, TUBE_Z])
        C = np.array([xb, ya, TUBE_Z]) if abs(xb - xa) > 0.1 and abs(yb - ya) > 0.1 else None
        pts = [A0, A1] + ([C] if C is not None else []) + [B1, B0]
        return pts

    def socket_items(self, src, dst, queued, t, rate_caps):
        pts = self.socket_path(src, dst)
        items = []
        horiz = pts[1:-1]
        lens = [np.linalg.norm(b - a) for a, b in zip(horiz[:-1], horiz[1:])]
        total = sum(lens)
        key_hi = max(p[0] + p[1] for p in pts)

        def draw(canvas):
            g = self.geo
            # Risers: brass.
            for a, b in ((pts[0], pts[1]), (pts[-2], pts[-1])):
                tube(canvas, g, a, b, TUBE_R * 0.9, TUBE_R * 0.9, BRASS, ks=1.1, rough=28)
            # Capsules first (inside the glass run).
            ncap = queued // KCAP
            def at(sd):
                acc = 0.0
                for (a, b), L in zip(zip(horiz[:-1], horiz[1:]), lens):
                    if sd <= acc + L:
                        return a + (b - a) * (sd - acc) / L, (b - a) / L
                    acc += L
                return horiz[-1], (horiz[-1] - horiz[-2]) / lens[-1]
            caps = []
            cl = 0.42
            for k in range(ncap):              # waiting at the receiving end
                caps.append(total - 0.35 - k * (cl + 0.06))
            # In transit: they travel then join the queue.
            for k in range(rate_caps):
                u = (t * 0.22 + k / rate_caps) % 1.0
                caps.append(0.3 + u * (total - 0.65 - ncap * (cl + 0.06)))
            for sd in caps:
                if sd < 0.2:
                    continue
                P, dv = at(sd)
                A, B = P - dv * cl / 2, P + dv * cl / 2
                tube(canvas, g, A, B, TUBE_R * 0.62, TUBE_R * 0.62, np.array([120, 58, 36], np.float32), ks=0.5, rough=14)
                for E in (A, B):
                    ball(canvas, g, E, TUBE_R * 0.62, BRASS * 0.9, ks=1.0)
            # Glass run with brass bands, elbows.
            for (a, b), L in zip(zip(horiz[:-1], horiz[1:]), lens):
                tube(canvas, g, a, b, TUBE_R, TUBE_R, GLASS * np.array([1.0, 0.95, 0.8]), mode="glass")
                nb = max(2, int(L / 1.6))
                dv = (b - a) / L
                for k in range(nb + 1):
                    P = a + dv * min(max(L * k / nb, 0.1), L - 0.1)
                    tube(canvas, g, P - dv * 0.07, P + dv * 0.07, TUBE_R * 1.25, TUBE_R * 1.25, BRASS, ks=1.1)
            for P in horiz:
                ball(canvas, g, P, TUBE_R * 1.3, BRASS, ks=1.2, rough=30)
            for P in (pts[0], pts[-1]):
                tube(canvas, g, P - np.array([0, 0, 0.1]), P + np.array([0, 0, 0.12]), TUBE_R * 1.4, TUBE_R * 1.4, BRASS)
        items.append((key_hi + 0.5, ("fn", draw)))
        return items

    # Garden hose (loopback TCP) ---------------------------------------------------------------

    def hose_curve(self):
        _, xa, ya, ma = TANKS[HOSE_ENDS[0]]
        _, xb, yb, mb = TANKS[HOSE_ENDS[1]]
        ra, rb = tank_r(ma), tank_r(mb)
        # Catmull-Rom through hand-placed control points with a lazy loop on the floor.
        ctrl = np.array([
            (xa - ra * 0.70, ya + ra * 0.70), (14.5, 9.9), (14.1, 11.2), (13.1, 11.7), (12.5, 11.0),
            (13.0, 10.3), (13.8, 10.8), (13.6, 11.9), (12.5, 12.3), (11.7, 11.5),
            (xb + rb * 0.62, yb + rb * 0.62)])
        pts = []
        P = np.vstack([ctrl[0], ctrl, ctrl[-1]])
        for i in range(1, len(P) - 2):
            p0, p1, p2, p3 = P[i - 1], P[i], P[i + 1], P[i + 2]
            for u in np.linspace(0, 1, 14, endpoint=False):
                u2, u3 = u * u, u * u * u
                pts.append(0.5 * ((2 * p1) + (-p0 + p2) * u + (2 * p0 - 5 * p1 + 4 * p2 - p3) * u2
                                  + (-p0 + 3 * p1 - 3 * p2 + p3) * u3))
        pts.append(P[-2])
        return np.array(pts)

    def hose_items(self, t):
        c = self.hose_curve()
        z = 0.16

        def draw(canvas):
            g = self.geo
            sp = np.array([g.p(x, y, z) for x, y in c])
            w = 0.15 * 2 * g.s * 1.05
            H, W = canvas.shape[:2]
            for width, col, off, op in ((w * 1.12, (14, 40, 16), 0.0, 1.0), (w, (44, 118, 50), 0.0, 1.0),
                                        (w * 0.62, (66, 160, 70), -0.12, 1.0), (w * 0.22, (190, 240, 170), -0.30, 0.8)):
                lay = K.Layer(W, H)
                lay.polyline(sp + np.array([0, off * w]), col, width=width)
                lay.onto(canvas, op)
            # Faint ribbing so it reads as a rubber garden hose.
            seg = np.hypot(*np.diff(sp, axis=0).T)
            arc = np.r_[0, np.cumsum(seg)]
            lay = K.Layer(W, H)
            for d0 in np.arange(0, arc[-1], w * 0.55):
                i = min(np.searchsorted(arc, d0), len(sp) - 2)
                p0, p1 = sp[i], sp[i + 1]
                tdir = (p1 - p0) / (np.linalg.norm(p1 - p0) + 1e-9)
                n = np.array([-tdir[1], tdir[0]])
                pm = p0 + tdir * (d0 - arc[i])
                lay.line(pm - n * w * 0.45, pm + n * w * 0.45, (20, 60, 24), width=max(1, w * 0.08))
            lay.onto(canvas, 0.45)
            for end in (c[0], c[-1]):
                ball(canvas, g, (end[0], end[1], z), 0.21, BRASS, ks=1.2)
        return [(24.0, ("fn", draw))]

    # Frame -------------------------------------------------------------------------------------

    def frame(self, t, status_lines, labels=True):
        g, ss = self.geo, self.ss
        canvas = self.static.copy()
        burst_run = [r for r in self.runs if r["state"] == "burst"][0]
        self.puddle(canvas, burst_run, t)
        items = []
        for name, (kind, x, y, mem) in TANKS.items():
            items.append((x + y, ("tank", name, t)))
        for run in self.runs:
            items += self.pipe_items(run, t)
        items.append((MANIFOLD[0] + MANIFOLD[1], ("manifold",)))
        items += self.hose_items(t)
        for (src, dst, q), rc in zip(SOCKETS, (2, 3)):
            items += self.socket_items(src, dst, q, t, rc)
        items.sort(key=lambda it: it[0])
        for _, it in items:
            self.draw_item(canvas, it)
        self.spray(canvas, burst_run, t)
        # Steam from the blocked tank's relief valve.
        self.steam(canvas, "browser-512", t)
        if t >= T_BURST:
            pass
        img = K.downsample(canvas, self.W, self.H)
        if labels:
            self.labels(img, t)
        S.status(img, list(status_lines), size=self.status_size)
        return img

    status_size = None

    def steam(self, canvas, name, t):
        g = self.geo
        _, x, y, mem = TANKS[name]
        r, h = tank_r(mem), tank_h(mem)
        rng = np.random.default_rng(12)
        for k in range(16):
            ph = (t * 0.45 + k / 16) % 1.0
            px, py = g.p(x, y, h + 0.5 + ph * 2.6)
            px += (math.sin(k * 1.7 + t * 1.3) * 0.5 + ph * 1.2) * g.s
            S.glow(canvas, px, py, (0.25 + ph * 0.8) * g.s, (200, 200, 205), 0.20 * (1 - ph) * min(1, ph * 6))

    def labels(self, img, t):
        draw = ImageDraw.Draw(img)
        g, ss = self.geo, self.ss
        compact = self.W < 1200
        sc = self.W / 1600.0
        mid, small = max(10, round(13 * sc)), max(9, round(12 * sc))
        if compact:
            sc, mid, small = 0.8, 12, 11
        def P(x, y, z=0.0):
            px, py = g.p(x, y, z)
            return px / ss, py / ss
        for name, (kind, x, y, mem) in TANKS.items():
            r, h = tank_r(mem), tank_h(mem)
            if name in PIPELINE:
                continue
            if name == "gzip-219":
                fade = float(np.clip(1 - (t - T_EXIT) / 0.7, 0, 1))
                px, py = P(x, y, h + 0.6)
                if fade > 0 and compact:
                    S.label(draw, px, py, name, size=small, fill=S.TEXT)
                elif fade > 0:
                    S.label(draw, px, py - 14 * sc, name, size=mid, fill=S.TEXT)
                    S.label(draw, px, py, fmt_mem(mem), size=small, fill=S.DIM)
                else:
                    px, py = P(x, y, 0)
                    S.label(draw, px, py + (22 if compact else 40 * sc), "gzip-219 exited", size=small, fill=(210, 170, 170))
                continue
            px, py = P(x, y, h + 0.65)
            if name in ("systemd-1", "journald-310", "nginx-128", "worker-134", "redis-207"):
                # Sockets come out of the top of these: label beside the riser.
                px, py = P(x, y, h * 0.55)
                px += (math.sqrt(2) * r * g.s / ss) + 6 * sc
                if compact:
                    S.label(draw, px, py, name, size=small, fill=S.TEXT, anchor="lm")
                    continue
                S.label(draw, px, py - 7 * sc, name, size=mid, fill=S.TEXT, anchor="lm")
                S.label(draw, px, py + 7 * sc, fmt_mem(mem), size=small, fill=S.DIM, anchor="lm")
                continue
            if compact:
                S.label(draw, px, py + 4, name, size=small, fill=S.TEXT)
                continue
            S.label(draw, px, py - 14 * sc, name, size=mid, fill=S.TEXT)
            S.label(draw, px, py, fmt_mem(mem), size=small, fill=S.DIM)
        # Pipeline caption over the chain of small tanks.
        a = P(*TANKS["cat-301"][1:3], 2.4)
        b = P(*TANKS["uniq-304"][1:3], 2.4)
        a = P(*TANKS["cat-301"][1:3], 0)
        b = P(*TANKS["uniq-304"][1:3], 0)
        S.label(draw, (a[0] + b[0]) / 2 - 120 * sc, (a[1] + b[1]) / 2 + 58 * sc,
                "cat access.log | grep 500 | sort | uniq -c", size=mid, fill=(236, 214, 170))
        for name in ([] if compact else PIPELINE):
            _, x, y, mem = TANKS[name]
            px, py = P(x, y, tank_h(mem) + 0.5)
            S.label(draw, px, py, name.split("-")[0], size=small, fill=S.TEXT)
        # Callouts.
        def callout(anchor, dx, dy, lines, col=(236, 220, 190)):
            ax, ay = anchor
            draw.line([(ax, ay), (ax + dx * 0.85, ay + dy * 0.85)], fill=(200, 200, 210), width=1)
            for i, (txt, c) in enumerate(lines[:1] if compact else lines):
                S.label(draw, ax + dx, ay + dy + i * 14 * sc, txt, size=small if i else mid,
                        fill=c, anchor="lm" if dx >= 0 else "rm")
        mx, my = MANIFOLD
        callout(P(mx, my, PIPE_Z + 0.5), *((34, -44) if compact else (-14 * sc, -62 * sc)),
                [("split unknown", (255, 214, 140)), ("3 pipes, 1/3 each", S.DIM)])
        br = [r for r in self.runs if r["state"] == "blocked"][0]
        (x0, y0), (x1, y1) = br["pts"][0], br["pts"][1]
        segs = list(zip(br["pts"][:-1], br["pts"][1:]))
        i = int(np.argmax([math.dist(a_, b_) for a_, b_ in segs]))
        (ax_, ay_), (bx_, by_) = segs[i]
        callout(P((ax_ + bx_) / 2, (ay_ + by_) / 2, PIPE_Z + 0.4), -30 * sc, -62 * sc,
                [("wchan pipe_write", (255, 150, 130)), ("64 KiB full, writer blocked", S.DIM)])
        bu = [r for r in self.runs if r["state"] == "burst"][0]
        a_, b_ = np.array(bu["pts"][-2]), np.array(bu["pts"][-1])
        L = np.linalg.norm(b_ - a_)
        Pb = a_ + (b_ - a_) / L * max(0.6, L * 0.45)
        if t >= T_BURST:
            callout(P(Pb[0], Pb[1], 1.2), 42 * sc, -40 * sc,
                    [("reader exited: EPIPE next", (150, 210, 255)), ("redis-207 still writing", S.DIM)])
        hc = self.hose_curve()
        mid_ = hc[len(hc) // 2 + 6]
        callout(P(mid_[0], mid_[1], 0.2), -60 * sc, -60 * sc,
                [("loopback TCP :5432", (150, 230, 140)), ("1.4 MB/s out, 220 KB/s back", S.DIM)])
        sp = self.socket_path("nginx-128", "worker-134")
        Q = (sp[1] + sp[2]) / 2 if len(sp) > 4 else sp[1]
        callout(P(Q[0], Q[1], Q[2]), 40 * sc, -50 * sc,
                [("unix socket", (236, 210, 140)), ("7 capsules = 28 KiB unread", S.DIM)])
        if compact:
            return
        so = [r for r in self.runs if r["state"] == "dry"][0]
        (a1, b1), (a2, b2) = so["pts"][0], so["pts"][-1]
        callout(P((a1 + a2) / 2, (b1 + b2) / 2, PIPE_Z + 0.3), 30 * sc, -44 * sc,
                [("dry: pipe_read", (190, 200, 220))])


def status_lines(t, wide):
    first = ("ISOTOP / PLUMBING / DEMO   192 processes | 16 tanks | 9 pipes, 2 unix sockets, 1 loopback TCP | "
             "K = 1 capsule per 4 KiB | wchar 61.3 MB/s")
    if wide:
        return [first,
                "tank = process (size ~ memory, band = kind) | copper = pipe or FIFO, water speed = writer's rate | "
                "brass tube = unix socket | green hose = loopback TCP | gauge = pipe fill, red = blocked",
                "Tab next view | v views | click a tank or pipe to inspect | Space pause | q quit   "
                f"wchan: 1 pipe_write, 1 pipe_read | 1 burst | t={63 + t:.1f}s"]
    return ["ISOTOP / PLUMBING / DEMO   192 procs | 16 tanks | 9 pipes, 2 unix sockets, 1 TCP | K = 1 capsule per 4 KiB",
            "tank = process (~memory) | copper = pipe, water speed = rate | brass tube = unix socket",
            f"green hose = loopback TCP | gauge = pipe fill, red = blocked | t={63 + t:.1f}s"]


def main(which):
    if which in ("still", "both"):
        sc = Scene(1600, 900, ss=2, tex_n=1600)
        sc.status_size = 12
        t = T_BURST + 1.1
        img = sc.frame(t, status_lines(t, True))
        print("still", S.save_still(img, "plumbing"))
    if which in ("anim", "both"):
        sc = Scene(960, 540, ss=2, tex_n=1200)
        fps, seconds = 12, 9
        frames = []
        for i in range(fps * seconds):
            t = i / fps
            frames.append(sc.frame(t, status_lines(t, False)))
            if i % 20 == 0:
                print("frame", i, flush=True)
        path, size = K.save_animation(frames, "plumbing", fps=fps)
        K.contact_sheet([frames[i] for i in (6, 40, 64, 100)], os.path.join(S.OUT, "plumbing-sheet.png"))
        print("gif", path, f"{size / 1e6:.2f} MB")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "both")
