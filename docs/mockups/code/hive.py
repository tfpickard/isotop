"""isotop view mockup: hive, an apiary of cgroups.

Hive = cgroup, frame = process, hex cell = 4 MiB of PSS, bee = thread.
Run: python3 -I hive.py [still|anim|both] [--frames a,b,c,d]
"""
import math
import sys

PROTO = "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto"
sys.path.insert(0, PROTO)

import numpy as np
from PIL import Image, ImageDraw, ImageFilter
from scipy import ndimage

import isostyle as S

CELL_MIB = 4
SQ3 = math.sqrt(3.0)

# ----------------------------------------------------------------------------- data (MiB)
# name, anon, file, shmem, swap, growth (young anon since the last sample), threads, cpu%
HIVES = [
    ("system.slice", "system", [
        ("postgres-41", 168, 196, 304, 12, 0, 1, 2.0),
        ("journald-310", 40, 112, 4, 0, 0, 1, 1.0),
        ("nginx-128", 28, 22, 10, 0, 0, 1, 0.4),
    ], []),
    ("user@1000.service", "session", [
        ("browser-512", 652, 220, 112, 48, 60, 48, 38.0),
        ("compiler-90", 236, 92, 4, 0, 160, 16, 610.0),
        ("language-server-71", 312, 60, 4, 36, 0, 22, 9.0),
    ], ["shell-170", "pipewire-95"]),
    ("docker-ab12.scope", "container", [
        ("redis-207", 400, 16, 0, 196, 0, 6, 4.0),
        ("worker-134", 196, 48, 8, 0, 24, 12, 145.0),
    ], []),
]

# cell kinds
EMPTY, HONEY, CAPPED, POLLEN, JELLY, BROOD = range(6)


def cells(mib):
    return int(round(mib / CELL_MIB))


# ----------------------------------------------------------------------------- comb layout
class Comb:
    """Hex grid inside a frame; positions in frame-local units of one cell pitch."""

    def __init__(self, cols, rows):
        self.cols, self.rows = cols, rows
        xs, ys = [], []
        for r in range(rows):
            for c in range(cols - (r % 2)):
                xs.append(c + 0.5 + 0.5 * (r % 2))
                ys.append(r * SQ3 / 2 + 1 / SQ3)
        self.x = np.array(xs)
        self.y = np.array(ys)
        self.w = cols
        self.h = (rows - 1) * SQ3 / 2 + 2 / SQ3
        self.n = len(xs)

    def layout(self, honey, capped, pollen, jelly, brood):
        """Beekeeper's convention: brood in the centre, pollen around it, royal jelly below,
        honey arching above and capped (swapped) honey on the rim of the arch."""
        cx, cy = self.w / 2, self.h * 0.58
        dx = (self.x - cx) / 1.55
        dy = self.y - cy
        d = np.sqrt(dx * dx + dy * dy)
        kind = np.full(self.n, EMPTY)
        order = np.zeros(self.n)
        free = np.ones(self.n, bool)

        def take(count, prio, k):
            if count <= 0:
                return
            idx = np.where(free)[0]
            idx = idx[np.argsort(prio[idx], kind="stable")][:count]
            kind[idx] = k
            free[idx] = False
            order[idx] = np.arange(len(idx))

        take(brood, d, BROOD)
        take(pollen, d, POLLEN)
        take(jelly, d - 0.9 * dy, JELLY)
        # honey grows as a dome over the nest: cells above count as near, cells below as far,
        # so a large store meets the top bar and spreads into the classic arch with shoulders
        dyh = np.where(dy < 0, dy * 0.42, dy * 2.6)
        up = np.sqrt(dx * dx * 0.8 + dyh * dyh)
        take(honey, up, HONEY)
        take(capped, up, CAPPED)
        return kind, order


# ----------------------------------------------------------------------------- drawing helpers
def hexagon(cx, cy, r):
    return [(cx + r * math.cos(math.pi / 6 + k * math.pi / 3), cy + r * math.sin(math.pi / 6 + k * math.pi / 3))
            for k in range(6)]


def shade(col, f):
    return tuple(int(max(0, min(255, c * f))) for c in col)


POLLEN_COLS = [(226, 104, 30), (204, 72, 34), (236, 128, 40), (196, 150, 40), (214, 88, 60), (180, 160, 60)]


def draw_cell(d, x, y, r, kind, rng_v, dim, grow=1.0):
    """One comb cell at screen (x, y) with pitch-derived radius r."""
    wall = r * 0.86
    if kind == EMPTY:
        d.polygon(hexagon(x, y, wall), fill=shade((70, 46, 24), dim))
        d.polygon(hexagon(x + r * 0.05, y + r * 0.08, wall * 0.62), fill=shade((40, 26, 14), dim))
        return
    if kind == HONEY:
        d.polygon(hexagon(x, y, wall), fill=shade((168, 92, 10), dim))
        d.polygon(hexagon(x + r * 0.05, y + r * 0.07, wall * 0.74), fill=shade((246, 178, 36), dim))
        d.polygon(hexagon(x + r * 0.1, y + r * 0.14, wall * 0.36), fill=shade((255, 214, 92), dim))
        d.ellipse([x - r * 0.46, y - r * 0.5, x - r * 0.14, y - r * 0.26], fill=shade((255, 240, 190), dim))
        return
    if kind == CAPPED and grow < 1.0:
        draw_cell(d, x, y, r, HONEY, rng_v, dim)
        g = max(0.0, grow)
        d.polygon(hexagon(x, y, wall * (0.35 + 0.65 * g)), fill=shade((222, 192, 112), dim * (1.0 + 0.25 * (1 - g))))
        return
    if kind == CAPPED:
        d.polygon(hexagon(x, y, wall), fill=shade((176, 140, 70), dim))
        d.polygon(hexagon(x - r * 0.04, y - r * 0.05, wall * 0.8), fill=shade((222, 192, 112), dim))
        d.polygon(hexagon(x - r * 0.1, y - r * 0.12, wall * 0.42), fill=shade((240, 218, 150), dim))
        return
    if kind == POLLEN:
        col = POLLEN_COLS[int(rng_v * len(POLLEN_COLS)) % len(POLLEN_COLS)]
        d.polygon(hexagon(x, y, wall), fill=shade((70, 46, 24), dim))
        d.polygon(hexagon(x, y + r * 0.02, wall * 0.8), fill=shade(col, dim * 0.85))
        d.ellipse([x - r * 0.3, y - r * 0.2, x + r * 0.1, y + r * 0.2], fill=shade(col, dim * 1.08))
        d.point([(x + r * 0.25, y + r * 0.25), (x - r * 0.35, y + r * 0.3)], fill=shade(col, dim * 0.55))
        return
    if kind == JELLY:
        d.polygon(hexagon(x, y, wall), fill=shade((70, 46, 24), dim))
        d.ellipse([x - wall * 0.72, y - wall * 0.66, x + wall * 0.72, y + wall * 0.74], fill=shade((238, 234, 220), dim))
        d.ellipse([x - r * 0.36, y - r * 0.38, x - r * 0.08, y - r * 0.16], fill=shade((255, 255, 244), dim))
        return
    if kind == BROOD:
        d.polygon(hexagon(x, y, wall), fill=shade((70, 46, 24), dim))
        g = max(0.0, min(1.0, grow))
        if g <= 0.02:
            return
        # a pearly C-shaped larva curled in the cell
        rr = wall * 0.66 * (0.45 + 0.55 * g)
        a0 = rng_v * 360
        d.pieslice([x - rr, y - rr, x + rr, y + rr], a0, a0 + 300, fill=shade((236, 232, 222), dim))
        d.ellipse([x - rr * 0.45, y - rr * 0.45, x + rr * 0.45, y + rr * 0.45], fill=shade((196, 188, 176), dim))
        d.ellipse([x - rr * 0.8, y - rr * 0.85, x - rr * 0.2, y - rr * 0.35], fill=shade((255, 255, 252), dim))


def ellipse_poly(cx, cy, a, b, ang, n=18):
    ca, sa = math.cos(ang), math.sin(ang)
    return [(cx + a * math.cos(t) * ca - b * math.sin(t) * sa, cy + a * math.cos(t) * sa + b * math.sin(t) * ca)
            for t in np.linspace(0, 2 * math.pi, n, endpoint=False)]


def draw_bee(d, x, y, ang, L, queen=False, flap=0.0, mark=None, dim=1.0):
    """Bee seen from above: abdomen striped, thorax, head, two translucent wings."""
    ca, sa = math.cos(ang), math.sin(ang)

    def at(u, v):  # u along the body (head positive), v across
        return x + u * ca - v * sa, y + u * sa + v * ca

    ab_len = L * (0.36 if not queen else 0.46)
    ab_c = at(-L * 0.18 - (0.06 * L if queen else 0), 0)
    d.polygon(ellipse_poly(ab_c[0] + L * 0.04, ab_c[1] + L * 0.06, ab_len, L * 0.2, ang), fill=(0, 0, 0, 70))
    d.polygon(ellipse_poly(*ab_c, ab_len, L * 0.19, ang), fill=shade((214, 150, 40), dim) + (255,),
              outline=shade((40, 26, 12), dim) + (255,))
    for k in range(3):
        u = -L * 0.08 - k * L * 0.16 - (0.08 * L if queen else 0)
        rel = (u - (-L * 0.18 - (0.06 * L if queen else 0))) / ab_len
        hw = L * 0.19 * math.sqrt(max(0.0, 1 - rel * rel)) * 0.96
        p0, p1 = at(u, -hw), at(u, hw)
        d.line([p0, p1], fill=shade((30, 20, 10), dim) + (255,), width=max(1, int(L * 0.1)))
    th = at(L * 0.12, 0)
    d.ellipse([th[0] - L * 0.13, th[1] - L * 0.13, th[0] + L * 0.13, th[1] + L * 0.13],
              fill=shade((120, 82, 40), dim) + (255,), outline=shade((40, 26, 12), dim) + (255,))
    hd = at(L * 0.31, 0)
    d.ellipse([hd[0] - L * 0.09, hd[1] - L * 0.09, hd[0] + L * 0.09, hd[1] + L * 0.09],
              fill=shade((30, 22, 16), dim) + (255,))
    if mark is not None:
        d.ellipse([th[0] - L * 0.06, th[1] - L * 0.06, th[0] + L * 0.06, th[1] + L * 0.06], fill=mark + (255,))
    spread = 0.55 + 0.35 * flap
    for side in (-1, 1):
        wa = ang + math.pi + side * spread
        wc = at(L * 0.06, side * L * 0.08)
        wx, wy = wc[0] + math.cos(wa) * L * 0.26, wc[1] + math.sin(wa) * L * 0.26
        d.polygon(ellipse_poly(wx, wy, L * 0.25, L * 0.1, wa), fill=(220, 232, 255, 70),
                  outline=(236, 242, 255, 120))


# ----------------------------------------------------------------------------- scene
class Cam:
    def __init__(self, W, H, ss, k, z=1.0, bx=0.0, by=0.0):
        self.ss = ss
        self.s = 72 * k * z * ss
        self.ox = (800 * k * z + bx) * ss
        self.oy = (300 * k * z + by) * ss
        self.W, self.H = int(W * ss), int(H * ss)

    def p(self, x, y, z=0.0):
        return (self.ox + (x - y) * self.s, self.oy + (x + y) * self.s * 0.5 - z * self.s)


PLOT = 8.2
HIVE_POS = [(-3.42, 0.0), (0.0, 0.8), (3.36, 0.0)]   # (u across the screen, forward)
C0 = 4.1
HIVE_TOP = 1.72


def hive_world(i):
    u, f = HIVE_POS[i]
    return C0 + u + f, C0 - u + f


def ground_layer(cam, rng):
    """Sky plus the plot: a dark meadow slab with clover specks and soft hive shadows."""
    W, H = cam.W, cam.H
    sky = S.sky(W, H, glow=(160, 70, 140), base=(10, 9, 22), centre=(0.16, 0.26))
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    a = (xx - cam.ox) / cam.s
    b = (yy - cam.oy) / (cam.s * 0.5)
    gx, gy = (a + b) / 2, (b - a) / 2
    inside = (gx >= 0) & (gx <= PLOT) & (gy >= 0) & (gy <= PLOT)
    n1 = ndimage.gaussian_filter(rng.standard_normal((256, 256)).astype(np.float32), 3, mode="wrap")
    n2 = ndimage.gaussian_filter(rng.standard_normal((256, 256)).astype(np.float32), 0.8, mode="wrap")
    n1 /= n1.std()
    n2 /= n2.std()
    ti = (np.clip(gx, 0, PLOT) / PLOT * 255).astype(int)
    tj = (np.clip(gy, 0, PLOT) / PLOT * 255).astype(int)
    tex = 0.6 * n1[tj, ti] + 0.4 * n2[tj, ti]
    grass = np.array((30, 46, 40), np.float32) * (0.85 + 0.12 * tex[..., None])
    # light falls off towards the back corner, isotop-style vignette on the plot
    far = 1 - np.clip((gx + gy) / (2 * PLOT), 0, 1)
    grass *= (0.78 + 0.32 * (1 - far))[..., None]
    out = np.where(inside[..., None], grass, sky)
    # slab sides
    T = 0.35
    for side in ("x", "y"):
        if side == "x":
            p0, p1 = cam.p(PLOT, 0), cam.p(PLOT, PLOT)
        else:
            p0, p1 = cam.p(0, PLOT), cam.p(PLOT, PLOT)
        poly = [p0, p1, (p1[0], p1[1] + T * cam.s), (p0[0], p0[1] + T * cam.s)]
        m = Image.new("L", (W, H), 0)
        ImageDraw.Draw(m).polygon(poly, fill=255)
        mm = (np.asarray(m, np.float32) / 255)[..., None]
        col = np.array((18, 20, 30) if side == "x" else (26, 28, 40), np.float32)
        out = out * (1 - mm) + col * mm
    # clover and tiny flowers
    img = S.to_image(out).convert("RGBA")
    d = ImageDraw.Draw(img, "RGBA")
    for _ in range(0):
        gxv, gyv = rng.uniform(0.2, PLOT - 0.2, 2)
        px, py = cam.p(gxv, gyv)
        r = cam.s * rng.uniform(0.012, 0.022)
        col = [(214, 210, 236), (196, 170, 230), (236, 226, 190), (120, 160, 110)][int(rng.integers(4))]
        alpha = int(30 + 70 * (gxv + gyv) / (2 * PLOT))
        d.ellipse([px - r, py - r * 0.6, px + r, py + r * 0.6], fill=col + (alpha,))
    # soft shadows under the hives
    sh = Image.new("L", (W, H), 0)
    sd = ImageDraw.Draw(sh)
    for i in range(3):
        hx, hy = hive_world(i)
        pts = [cam.p(hx - 0.95, hy - 1.05), cam.p(hx + 1.25, hy - 1.05), cam.p(hx + 1.25, hy + 1.35),
               cam.p(hx - 0.95, hy + 1.35)]
        sd.polygon(pts, fill=150)
    sh = sh.filter(ImageFilter.GaussianBlur(cam.s * 0.18))
    pm = Image.new("L", (W, H), 0)
    ImageDraw.Draw(pm).polygon([cam.p(0, 0), cam.p(PLOT, 0), cam.p(PLOT, PLOT), cam.p(0, PLOT)], fill=255)
    sh = Image.fromarray((np.asarray(sh, np.float32) * np.asarray(pm, np.float32) / 255).astype(np.uint8))
    dark = Image.new("RGBA", (W, H), (4, 6, 10, 255))
    img = Image.composite(dark, img, sh)
    return img


def box_faces(cam, x0, x1, y0, y1, z0, z1):
    """Visible faces of an axis-aligned box: top, +y (left), +x (right)."""
    P = cam.p
    top = [P(x0, y0, z1), P(x1, y0, z1), P(x1, y1, z1), P(x0, y1, z1)]
    left = [P(x0, y1, z0), P(x1, y1, z0), P(x1, y1, z1), P(x0, y1, z1)]
    right = [P(x1, y0, z0), P(x1, y1, z0), P(x1, y1, z1), P(x1, y0, z1)]
    return top, left, right


def draw_box(d, cam, x0, x1, y0, y1, z0, z1, col, top=1.0, lf=0.8, rf=0.6, outline=None):
    t, l, r = box_faces(cam, x0, x1, y0, y1, z0, z1)
    d.polygon(l, fill=shade(col, lf) + (255,))
    d.polygon(r, fill=shade(col, rf) + (255,))
    d.polygon(t, fill=shade(col, top) + (255,))
    if outline:
        for poly in (l, r, t):
            d.line(poly + [poly[0]], fill=outline, width=1)


def draw_hive(img, cam, i, kind_col, rng):
    d = ImageDraw.Draw(img, "RGBA")
    hx, hy = hive_world(i)
    ax, ay = 0.66, 0.84                    # half extents (x depth, y width)
    x0, x1, y0, y1 = hx - ax, hx + ax, hy - ay, hy + ay
    wood = (176, 132, 84)
    # stand: four legs and a rail
    for (lx, ly) in ((x1 - 0.1, y1 - 0.1), (x1 - 0.1, y0 + 0.1), (x0 + 0.1, y1 - 0.1)):
        draw_box(d, cam, lx - 0.06, lx + 0.06, ly - 0.06, ly + 0.06, 0, 0.3, (52, 42, 36))
    draw_box(d, cam, x0 - 0.05, x1 + 0.05, y0 - 0.05, y1 + 0.05, 0.3, 0.38, (70, 56, 44))
    # landing board on the +y side
    draw_box(d, cam, x0 + 0.2, x1 - 0.2, y1, y1 + 0.3, 0.34, 0.4, (120, 92, 62))
    z = 0.38
    for hgt, name in ((0.72, "brood"), (0.46, "super")):
        draw_box(d, cam, x0, x1, y0, y1, z, z + hgt, wood, top=1.08, lf=0.92, rf=0.68)
        # plank seams on both visible faces
        P = cam.p
        for f in (0.33, 0.66):
            zz = z + hgt * f
            d.line([P(x0, y1, zz), P(x1, y1, zz)], fill=shade(wood, 0.62) + (255,), width=max(1, int(cam.s * 0.008)))
            d.line([P(x1, y0, zz), P(x1, y1, zz)], fill=shade(wood, 0.46) + (255,), width=max(1, int(cam.s * 0.008)))
        # hand holds
        hz = z + hgt * 0.7
        hc = P(hx, y1, hz)
        d.rounded_rectangle([hc[0] - cam.s * 0.22, hc[1] - cam.s * 0.05, hc[0] + cam.s * 0.22, hc[1] + cam.s * 0.05],
                            radius=cam.s * 0.04, fill=shade(wood, 0.45) + (255,))
        # kind-coloured trim band along the top edge
        draw_box(d, cam, x0 - 0.015, x1 + 0.015, y0 - 0.015, y1 + 0.015, z + hgt - 0.1, z + hgt, kind_col,
                 top=1.0, lf=0.9, rf=0.7)
        z += hgt
    # entrance slot at the bottom of the brood box
    P = cam.p
    e = [P(hx - 0.38, y1 + 0.005, 0.4), P(hx + 0.38, y1 + 0.005, 0.4), P(hx + 0.38, y1 + 0.005, 0.47),
         P(hx - 0.38, y1 + 0.005, 0.47)]
    d.polygon(e, fill=(16, 10, 6, 255))
    # telescoping lid
    draw_box(d, cam, x0 - 0.07, x1 + 0.07, y0 - 0.07, y1 + 0.07, z, z + 0.14, (138, 146, 160), top=1.0, lf=0.8,
             rf=0.58)
    return z + 0.14


def lift_beam(img, cam, i, col):
    """A faint column of the hive's colour rising from the lid to the lifted frames."""
    hx, hy = hive_world(i)
    x0, y0, x1, y1 = frame_rect(cam, i, 0)
    lid = cam.p(hx, hy, HIVE_TOP)
    half = (0.84 + 0.66) * cam.s * 0.8
    top_y, bot_y = y1, lid[1]
    W, H = img.size
    yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
    f = np.clip((yy - top_y) / max(bot_y - top_y, 1), 0, 1)
    width = half * (0.7 + 0.3 * f)
    edge = np.clip(1 - np.abs(xx - lid[0]) / width, 0, 1) ** 0.7
    a = edge * (0.07 + 0.3 * f ** 1.5) * ((yy >= top_y) & (yy <= bot_y))
    c = np.asarray(img, np.float32)
    c = c * (1 - a[..., None]) + np.array(col, np.float32) * a[..., None]
    return Image.fromarray(np.clip(c, 0, 255).astype(np.uint8))


def frame_rect(cam, i, j):
    """Screen rectangle of frame j of hive i (frames face the viewer, lifted and fanned)."""
    hx, hy = hive_world(i)
    back = 0.5 + 0.4 * j
    lat = 0.1 * j
    fx, fy = hx - back + lat, hy - back - lat
    zb = HIVE_TOP + 0.5 + 0.78 * j
    bx, by = cam.p(fx, fy, zb)
    w, h = 4.4 * cam.s, 2.04 * cam.s
    return bx - w / 2, by - h, bx + w / 2, by


class Frame:
    def __init__(self, hive, j, spec, kind):
        name, anon, file, shmem, swap, growth, threads, cpu = spec
        self.hive, self.j, self.name, self.kind = hive, j, name, kind
        self.anon, self.file, self.shmem, self.swap, self.growth = anon, file, shmem, swap, growth
        self.threads, self.cpu = threads, cpu
        self.counts = dict(honey=cells(anon - growth), capped=cells(swap), pollen=cells(file), jelly=cells(shmem),
                           brood=cells(growth))
        self.total = sum(self.counts.values())
        self.comb = Comb(28, 13)
        self.kinds, self.order = self.comb.layout(**self.counts)
        rng = np.random.default_rng(hash(name) % 2 ** 32)
        self.rv = rng.uniform(0, 1, self.comb.n)


def build():
    frames = []
    for i, (hname, kind, procs, hidden) in enumerate(HIVES):
        for j, spec in enumerate(procs):
            frames.append(Frame(i, j, spec, kind))
    return frames


class Bees:
    """Threads: busy bees scurry at a speed set by per-thread CPU, idle ones cluster still."""

    def __init__(self, frame, rng):
        n = min(frame.threads, 200)
        self.n = n
        share = frame.cpu / 100.0
        w = rng.dirichlet(np.ones(n) * 0.5) if n > 1 else np.array([1.0])
        self.cpu = np.clip(share * w, 0, 1.0)
        if frame.name == "compiler-90":
            self.cpu = np.clip(rng.uniform(0.25, 0.6, n), 0, 1)
        comb = frame.comb
        self.cw, self.ch = comb.w, comb.h
        busy = self.cpu > 0.03
        cx, cy = comb.w / 2, comb.h * 0.58
        self.x = np.where(busy, rng.uniform(1.0, comb.w - 1.0, n), cx - 6.0 + rng.normal(0, 1.6, n))
        self.y = np.where(busy, rng.uniform(1.0, comb.h - 1.0, n), cy - 2.2 + rng.normal(0, 1.0, n))
        self.ang = rng.uniform(0, 2 * math.pi, n)
        self.turn = rng.normal(0, 1, n)
        self.speed = 0.25 + 9.0 * np.sqrt(self.cpu)       # cells per second
        self.speed[~busy] = 0.0
        self.rng = rng
        self.queen = 0
        self.x[0], self.y[0] = cx + 0.4, cy + 0.2
        self.speed[0] = min(self.speed[0], 0.6)

    def step(self, dt):
        self.turn += self.rng.normal(0, 1.4, self.n) * dt * 4
        self.turn *= 0.92
        self.ang += self.turn * dt * 2.2
        self.x += np.cos(self.ang) * self.speed * dt
        self.y += np.sin(self.ang) * self.speed * dt
        m = 0.7
        for arr, lim, flip in ((self.x, self.cw, 0), (self.y, self.ch, 1)):
            lo, hi = arr < m, arr > lim - m
            if flip == 0:
                self.ang[lo | hi] = math.pi - self.ang[lo | hi]
            else:
                self.ang[lo | hi] = -self.ang[lo | hi]
            np.clip(arr, m, lim - m, out=arr)


def frame_geometry(cam, fr):
    x0, y0, x1, y1 = frame_rect(cam, fr.hive, fr.j)
    s = cam.s
    top_bar = 0.13 * s
    side = 0.085 * s
    bot = 0.07 * s
    ix0, iy0, ix1, iy1 = x0 + side, y0 + top_bar, x1 - side, y1 - bot
    pitch = (ix1 - ix0) / fr.comb.w
    oy = iy0 + ((iy1 - iy0) - fr.comb.h * pitch) / 2
    return (x0, y0, x1, y1), (ix0, iy0, ix1, iy1), pitch, oy


def draw_frame(img, cam, fr, kinds, grow, dim, kind_col):
    d = ImageDraw.Draw(img, "RGBA")
    (x0, y0, x1, y1), (ix0, iy0, ix1, iy1), pitch, oy = frame_geometry(cam, fr)
    s = cam.s
    wood = (196, 156, 104)
    lug = 0.16 * s
    # thickness seen from above: a thin strip over the top bar
    d.polygon([(x0 - lug, y0), (x1 + lug, y0), (x1 + lug + 0.0, y0 - 0.05 * s), (x0 - lug, y0 - 0.05 * s)],
              fill=shade(wood, dim * 1.12) + (255,))
    # glow of the kind colour around the frame, like isotop's selection halo
    d.rectangle([x0, y0, x1, y1], fill=shade(wood, dim * 0.85) + (255,))
    d.rectangle([x0 - lug, y0, x1 + lug, y0 + 0.13 * s], fill=shade(wood, dim) + (255,))
    d.line([(x0 - lug, y0 + 0.13 * s), (x1 + lug, y0 + 0.13 * s)], fill=shade(wood, dim * 0.6) + (255,),
           width=max(1, int(s * 0.012)))
    # kind-coloured tag on the top bar (beekeepers mark frames)
    d.rectangle([x0 + 0.05 * s, y0 + 0.035 * s, x0 + 0.42 * s, y0 + 0.095 * s], fill=shade(kind_col, dim) + (255,))
    # comb foundation (wax walls)
    d.rectangle([ix0, iy0, ix1, iy1], fill=shade((150, 104, 48), dim) + (255,))
    r = pitch / SQ3
    comb = fr.comb
    for k in range(comb.n):
        cx = ix0 + comb.x[k] * pitch
        cy = oy + comb.y[k] * pitch
        draw_cell(d, cx, cy, r, kinds[k], fr.rv[k], dim, grow[k])
    # inner shadow under the top bar
    d.rectangle([ix0, iy0, ix1, iy0 + 0.035 * s], fill=(0, 0, 0, 70))


def kinds_at(fr, t, anim):
    """Cell kinds and brood growth for time t (seconds). anim False: the still state."""
    kinds = fr.kinds.copy()
    grow = np.ones(fr.comb.n)
    if not anim:
        if fr.name == "compiler-90":
            # the newest brood rows are still small larvae
            b = np.where(kinds == BROOD)[0]
            o = fr.order[b]
            grow[b] = np.clip(1.3 - o / max(o.max(), 1) * 0.9, 0.25, 1)
        return kinds, grow, 0, 0
    newb = newc = 0
    if fr.name == "compiler-90":
        b = np.where(kinds == BROOD)[0]
        nb = len(b)
        start = int(nb * 0.35)
        # brood cells appear in order as RSS grows, then the larvae fatten
        for k in b:
            o = fr.order[k]
            if o < start:
                continue
            t_on = 0.4 + (o - start) / max(nb - start, 1) * 7.0
            g = (t - t_on) / 1.2
            if g <= 0:
                kinds[k] = EMPTY
            else:
                grow[k] = min(1.0, g)
                newb += 1
        newb += start
    if fr.name == "redis-207":
        c = np.where(kinds == CAPPED)[0]
        nc = len(c)
        first = int(nc * 0.25)
        for k in c:
            o = fr.order[k]
            if o < first:
                continue
            t_cap = 1.0 + (o - first) / max(nc - first, 1) * 7.0
            if t < t_cap:
                kinds[k] = HONEY
            else:
                grow[k] = min(1.0, (t - t_cap) / 0.7)
                newc += 1
        newc += first
    return kinds, grow, newb, newc


# ----------------------------------------------------------------------------- overlays
def legend(img, k, x, y):
    d = ImageDraw.Draw(img, "RGBA")
    sz = max(9, round(12 * k))
    f = S.font(sz)
    fb = S.font(sz, S.FONT_MONO_BOLD)
    rows = [(HONEY, "honey", "Pss_Anon"), (POLLEN, "pollen", "Pss_File"), (JELLY, "royal jelly", "Pss_Shmem"),
            (CAPPED, "capped", "SwapPss"), (BROOD, "brood", "RSS growth")]
    lh = int(sz * 1.6)
    w = int(sz * 20.5)
    h = lh * (len(rows) + 2) + int(sz * 0.9)
    x = x - w
    d.rounded_rectangle([x, y, x + w, y + h], radius=5, fill=(12, 12, 20, 205), outline=(90, 86, 110, 200))
    d.text((x + sz * 0.8, y + sz * 0.6), f"1 cell = {CELL_MIB} MiB PSS", font=fb, fill=(236, 226, 196))
    for i, (kind, nm, src) in enumerate(rows):
        cy = y + sz * 0.6 + lh * (i + 1) + sz * 0.6
        cx = x + sz * 1.6
        r = sz * 0.62
        d.polygon(hexagon(cx, cy, r * 1.05), fill=(150, 104, 48))
        draw_cell(d, cx, cy, r, kind, 0.1, 1.0)
        d.text((cx + sz * 1.2, cy), nm, font=f, fill=(214, 216, 228), anchor="lm")
        d.text((cx + sz * 10.2, cy), src, font=f, fill=(150, 156, 176), anchor="lm")
    cy = y + sz * 0.6 + lh * (len(rows) + 1) + sz * 0.6
    draw_bee(d, x + sz * 1.6, cy, -0.4, sz * 1.5)
    d.text((x + sz * 2.8, cy), "bee = thread, speed = CPU", font=f, fill=(214, 216, 228), anchor="lm")


def status_lines(short=False):
    if short:
        return ["ISOTOP / HIVE / DEMO   192 processes | 3 hives | 8 frames | 107 bees | RAM 20.0 / 32.0 GiB",
                "hive = cgroup | frame = process | 1 cell = 4 MiB PSS: honey anon, pollen file, jelly shmem,",
                "capped = swap, brood = RSS growth | bee = thread, speed = CPU, queen = leader"]
    return ["ISOTOP / HIVE / DEMO   192 processes | 3 hives | 8 frames shown, 184 counted | 107 bees | "
            "RAM 20.0 GiB / 32.0 GiB | swap 0.2 GiB",
            "Hive = cgroup | frame = process (top 3 by RSS) | 1 cell = 4 MiB PSS: honey = Pss_Anon, pollen = Pss_File, "
            "royal jelly = Pss_Shmem, capped = SwapPss",
            "brood = RSS growth since the last sample | bee = thread, speed = per-thread CPU, queen = thread group leader"
            " | Tab next view | ? help | q quit"]


def render(frames, bees, W, H, t, anim, cache, view=(1.0, 0.0, 0.0)):
    k = W / 1600
    ss = 2
    lines = status_lines(short=W < 1200)
    sz = max(11, W // 115)
    band = int(sz * 1.45) * len(lines) + 10
    scene_h = H - band
    cam = Cam(W, scene_h, ss, k, *view)
    lab_k = k
    key = (W, H)
    if key not in cache:
        rng = np.random.default_rng(7)
        base = ground_layer(cam, rng)
        base = base.convert("RGB")
        for i, (hname, kind, procs, hidden) in enumerate(HIVES):
            base = lift_beam(base, cam, i, S.KIND[kind])
        tops = []
        for i, (hname, kind, procs, hidden) in enumerate(HIVES):
            tops.append(draw_hive(base, cam, i, S.KIND[kind], rng))
        cache[key] = (base, tops)
    base, tops = cache[key]
    img = base.copy()
    d = ImageDraw.Draw(img, "RGBA")
    # frames back to front: higher j first within a hive; hives left, right, then the forward middle one
    order = sorted(frames, key=lambda fr: (-fr.j, HIVE_POS[fr.hive][1], fr.hive))
    info = []
    for fr in order:
        dim = 1.0 - 0.16 * fr.j - (0.1 if fr.hive != 1 else 0.0)
        kinds, grow, nb, nc = kinds_at(fr, t, anim)
        kind_col = S.KIND[fr.kind]
        # lift lines from the hive to the frame lugs (the frame is pulled up out of its hive)
        (x0, y0, x1, y1), _, pitch, oy = frame_geometry(cam, fr)
        draw_frame(img, cam, fr, kinds, grow, dim, kind_col)
        bz = bees[fr.name]
        d = ImageDraw.Draw(img, "RGBA")
        ix0 = frame_geometry(cam, fr)[1][0]
        for b in range(bz.n):
            px = ix0 + bz.x[b] * pitch
            py = oy + bz.y[b] * pitch
            queen = b == 0
            L = pitch * (1.75 if not queen else 2.4)
            flap = math.sin(t * 40 + b * 1.7) if bz.speed[b] > 0.5 else 0.0
            draw_bee(d, px, py, bz.ang[b], L, queen=queen, flap=flap,
                     mark=kind_col if queen else None, dim=min(1.0, dim + 0.1))
        info.append((fr, (x0, y0, x1, y1), nb, nc, kinds))
    small = img.convert("RGB").resize((W, scene_h), Image.LANCZOS)
    full = Image.new("RGB", (W, H), S.PANEL)
    full.paste(small, (0, 0))
    ov = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(ov)
    lab = max(10, round(13 * k))
    for fr, (x0, y0, x1, y1), nb, nc, kinds in info:
        x0, y0, x1, y1 = (v / ss for v in (x0, y0, x1, y1))
        col = tuple(int(c) for c in S.KIND[fr.kind])
        n_cells = int(np.sum(kinds != EMPTY))
        if fr.hive == 2:
            tx, anc = x1 + 14 * k + 4, "lm"
        else:
            tx, anc = x0 - 14 * k - 4, "rm"
        ty = y0 + 10 * k
        S.label(d, tx, ty, fr.name, size=lab, fill=col, anchor=anc)
        S.label(d, tx, ty + lab * 1.3, f"{n_cells} cells  {fr.threads} bee{'s' if fr.threads > 1 else ''}",
                size=max(9, lab - 2), fill=(170, 174, 192), anchor=anc)
    for i, (hname, kind, procs, hidden) in enumerate(HIVES):
        hx, hy = hive_world(i)
        px, py = cam.p(hx + 1.15, hy + 1.15, 0)
        px, py = px / ss, py / ss
        col = tuple(int(c) for c in S.KIND[kind])
        if py + 14 * k + lab * 2 > scene_h or px < 40 or px > W - 40:
            continue
        S.label(d, px, py + 14 * k, hname, size=max(10, round(14 * k)), fill=col)
        more = f"+{len(hidden)} more: {', '.join(hidden)}" if hidden else f"{len(procs)} processes"
        S.label(d, px, py + 14 * k + lab * 1.35, more, size=max(9, lab - 2), fill=(150, 156, 176))
    full.paste(ov, (0, 0), ov)
    legend_img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    if view[0] == 1.0:
        legend(legend_img, k, int(W - 12 * k), int(12 * k))
    else:
        sz_l = max(9, round(12 * k))
        legend(legend_img, k, int(W * 0.588), int(scene_h - 8 - (int(sz_l * 1.6) * 7 + int(sz_l * 0.9))))
    full.paste(legend_img, (0, 0), legend_img)
    S.status(full, lines, size=sz)
    return full


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "still"
    frames = build()
    if mode in ("still", "both"):
        rng = np.random.default_rng(3)
        bees = {fr.name: Bees(fr, rng) for fr in frames}
        for _ in range(30):
            for b in bees.values():
                b.step(1 / 12)
        img = render(frames, bees, 1600, 900, 0.0, False, {})
        print("still", S.save_still(img, "hive"))
    if mode in ("anim", "both"):
        fps, dur = 12, 9.0
        n = int(fps * dur)
        picks = None
        if "--frames" in sys.argv:
            picks = [int(v) for v in sys.argv[sys.argv.index("--frames") + 1].split(",")]
        rng = np.random.default_rng(3)
        bees = {fr.name: Bees(fr, rng) for fr in frames}
        for _ in range(30):
            for b in bees.values():
                b.step(1 / 12)
        out, cache = [], {}
        for fi in range(n):
            t = fi / fps
            if picks is None or fi in picks:
                out.append(render(frames, bees, 960, 540, t, True, cache, view=(1.45, -425.0, -91.0)))
            for b in bees.values():
                b.step(1 / fps)
            if picks is not None and fi >= max(picks):
                break
        if picks is not None:
            W, H = 960, 540
            sheet = Image.new("RGB", (W * 2, H * 2))
            for j, f in enumerate(out[:4]):
                sheet.paste(f, ((j % 2) * W, (j // 2) * H))
            sheet.save(S.OUT + "/hive-sheet.png")
            print("sheet")
        else:
            print(S.save_frames(out, "hive", fps=fps))


if __name__ == "__main__":
    main()
