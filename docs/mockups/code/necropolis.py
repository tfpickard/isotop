"""isotop view mockup: necropolis, a graveyard of exited processes.

Run: python3 -I necropolis.py [still|anim|both] [--fast]
"""
import math
import sys
from datetime import datetime, timezone

PROTO = "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto"
sys.path.insert(0, PROTO)

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

import isostyle as S
import nha_iso as E

# ----------------------------------------------------------------------------- data
DEMO_TIME = datetime(2026, 10, 10, 4, 12, tzinfo=timezone.utc)
SYNODIC = 29.530588853
NEW_MOON = datetime(2000, 1, 6, 18, 14, tzinfo=timezone.utc)


def moon_age(t):
    days = (t - NEW_MOON).total_seconds() / 86400.0
    return days % SYNODIC


def moon_phase_name(age):
    lit = (1 - math.cos(2 * math.pi * age / SYNODIC)) / 2
    waxing = age < SYNODIC / 2
    if lit < 0.01:
        return "new moon"
    if lit > 0.99:
        return "full moon"
    if abs(lit - 0.5) < 0.02:
        return "first quarter" if waxing else "last quarter"
    return ("waxing " if waxing else "waning ") + ("crescent" if lit < 0.5 else "gibbous")


X, Y = 30.0, 26.0           # cemetery plot, world units
ROW_DX = 3.1
ROW0_X = 23.6
NROWS = 8
PATH_Y = (11.3, 12.9)       # cross path
MAUS_Y = (20.0, 24.6)       # mausoleum lot at the left end of rows 1 and 2

STONE_TINT = {
    "kernel": (104, 112, 132),   # slate
    "system": (112, 138, 134),   # green granite
    "session": (166, 142, 108),  # sandstone
    "container": (134, 118, 160),  # lilac marble
}

NAMES = {
    "session": ["gcc", "cc1", "ld", "as", "git", "python3", "node", "bash", "vim", "grep", "rg", "sed",
                "cargo", "rustc", "chrome", "less", "ssh", "make", "cmake", "pytest", "jq", "curl"],
    "system": ["cron", "logrotate", "man-db", "apt-check", "systemd-tmpfiles", "fwupd", "sshd",
               "NetworkManager-d", "polkit-agent", "updatedb", "cupsd", "snapd"],
    "kernel": ["kworker/u16:3", "kworker/2:1", "kworker/7:0", "kworker/u17:0"],
    "container": ["redis-cli", "entrypoint.sh", "nginx", "postgres", "pg_dump", "healthcheck", "sh"],
}


def human_time(sec):
    if sec < 1:
        return f"{sec * 1000:.0f} ms"
    if sec < 60:
        return f"{sec:.1f} s"
    if sec < 3600:
        return f"{int(sec // 60)} m {int(sec % 60)} s"
    return f"{int(sec // 3600)} h {int(sec % 3600 // 60)} m"


class Grave:
    def __init__(self, name, pid, kind, cpu_ms, rss_mib, cause, lifetime, shape="round"):
        self.name, self.pid, self.kind = name, pid, kind
        self.cpu_ms, self.rss_mib, self.cause, self.lifetime = cpu_ms, rss_mib, cause, lifetime
        self.shape = shape
        self.h = 0.55 + 0.36 * math.log10(max(cpu_ms, 3.0))       # lifetime CPU, log
        self.w = 0.62 + 0.15 * math.log2(max(rss_mib, 1.0))       # peak RSS, log2
        self.x = self.y0 = self.y1 = 0.0
        self.row = 0
        self.rise = 1.0


def make_rows(rng):
    """Rows of graves, row 0 newest. Returns list of lists of Grave."""
    rows = []
    kinds = ["session"] * 6 + ["system"] * 3 + ["container"] * 2 + ["kernel"]
    pid = 9000
    causes = ["exited peacefully"] * 7 + ["exited 1", "killed by SIGTERM", "exited 2", "killed by SIGPIPE",
                                          "killed by SIGINT"]
    for r in range(NROWS):
        row = []
        y = 1.3 + rng.uniform(0, 0.5)
        while True:
            kind = kinds[rng.integers(len(kinds))]
            name = NAMES[kind][rng.integers(len(NAMES[kind]))]
            cpu = float(10 ** rng.uniform(1.0, 6.2))
            rss = float(2 ** rng.uniform(1.0, 9.5))
            pid -= int(rng.integers(3, 90))
            shape = rng.choice(["round", "round", "round", "slab", "gothic", "round", "cross"],
                               p=[0.2, 0.2, 0.15, 0.17, 0.13, 0.1, 0.05])
            g = Grave(f"{name}-{pid}", pid, kind, cpu, rss, causes[rng.integers(len(causes))],
                      cpu / 1000 * rng.uniform(1.2, 30), shape)
            if g.h > 2.55 and rng.uniform() < 0.6:
                g.shape = "obelisk"
            if y + g.w > Y - 1.2:
                break
            # leave the cross path and the mausoleum lot free
            if y < PATH_Y[1] and y + g.w > PATH_Y[0]:
                y = PATH_Y[1] + 0.25
                continue
            if r in (1, 2) and y + g.w > MAUS_Y[0] - 0.3:
                break
            g.y0, g.y1 = y, y + g.w
            g.row = r
            row.append(g)
            y += g.w + rng.uniform(0.45, 0.95)
        rows.append(row)
    return rows


def special_graves(rows):
    """Plant the hover examples and the build burst."""
    # chrome-812: a long-lived, OOM-killed browser in row 1, right-hand side
    row = rows[1]
    big = sorted(row, key=lambda g: abs((g.y0 + g.y1) / 2 - 5.0))[0]
    big.name, big.pid, big.kind = "chrome-812", 812, "session"
    big.cpu_ms, big.rss_mib, big.cause = 3.1e6, 6100, "taken by the OOM killer"
    big.lifetime = 3 * 3600 + 12 * 60
    big.h = 0.55 + 0.36 * math.log10(big.cpu_ms)
    big.shape = "obelisk"
    return big


def burst_row(rng):
    """The build that just finished: make-77, its children and one mass grave."""
    row = []
    # (name, kind, lifetime cpu ms, peak rss MiB, cause, lifetime s, shape, y0)
    spec = [
        ("ld-4471", "session", 2.9e3, 610, "exited peacefully", 3.4, "round", 1.6),
        ("cc1-4433", "session", 1.6e3, 240, "exited peacefully", 1.9, "gothic", 4.3),
        ("MASS", "session", 0, 0, "", 0, "mass", 6.9),
        ("as-4460", "session", 90.0, 18, "exited peacefully", 0.2, "round", 13.4),
        ("make-77", "session", 4.4e4, 96, "dumped core", 1310.0, "slab", 15.3),
        ("cc1-4452", "session", 1.9e3, 300, "exited peacefully", 2.2, "round", 17.55),
        ("gcc-4411", "session", 2.1e3, 180, "exited peacefully", 2.3, "round", 20.0),
        ("ninja-73", "session", 6.1e3, 40, "exited 1", 52.0, "round", 22.45),
    ]
    for name, kind, cpu, rss, cause, life, shape, y in spec:
        if shape == "mass":
            g = Grave("cc1 x 31  (make-77)", 0, kind, 3e4, 30, "31 children within 0.4 s", 0, "mass")
            g.h, g.w = 0.42, 3.4
        else:
            g = Grave(name, int(name.split("-")[-1]), kind, cpu, rss, cause, life, shape)
        g.y0, g.y1 = y, y + g.w
        row.append(g)
    return row


# ----------------------------------------------------------------------------- geometry
M_GROUND, M_PLOT, M_STONE, M_ENGRAVE, M_SOIL, M_PIT, M_DIRT, M_ZOMBIE, M_MARKER, M_IRON, M_MAUS, \
    M_DOOR, M_TREE, M_PATHSTONE = range(14)


def stone_geom(sc, g, x_off, z_scale=1.0, sink=0.0):
    """Headstone polygons. g.x is the stone centre line, y0..y1 its width."""
    tint = np.array(STONE_TINT[g.kind], np.float32)
    h = g.h * z_scale
    if h < 0.02:
        return
    thick = 0.34 + 0.06 * min(g.w, 2.5)
    xc = g.x + x_off
    x0, x1 = xc - thick / 2, xc + thick / 2
    y0, y1 = g.y0, g.y1
    w = y1 - y0
    yc = (y0 + y1) / 2
    key = xc + yc
    z0 = -sink
    if g.shape == "mass":
        # a long low slab with a raised lid
        sc.box(xc - 0.9, xc + 0.9, y0, y1, z0, z0 + h * 0.6, tint * 0.92, M_STONE, key)
        sc.box(xc - 0.8, xc + 0.8, y0 + 0.1, y1 - 0.1, z0 + h * 0.6, z0 + h, tint, M_STONE, key + 0.001)
        return
    if g.shape in ("slab", "obelisk", "cross"):
        pl = 0.16 * min(z_scale * 3, 1)
        sc.box(x0 - 0.14, x1 + 0.14, y0 - 0.12, y1 + 0.12, z0, z0 + pl, tint * 0.85, M_STONE, key - 0.01)
        z0 += pl
    if g.shape == "obelisk":
        a0 = min(w, 1.2) * 0.5
        a1 = a0 * 0.62
        cx, cy = xc, yc
        top = z0 + h * 0.9
        # +y face
        sc.poly([(cx - a0, cy + a0, z0), (cx + a0, cy + a0, z0), (cx + a1, cy + a1, top), (cx - a1, cy + a1, top)],
                tint, M_STONE, key)
        sc.poly([(cx + a0, cy - a0, z0), (cx + a0, cy + a0, z0), (cx + a1, cy + a1, top), (cx + a1, cy - a1, top)],
                tint, M_STONE, key)
        apex = (cx, cy, z0 + h)
        sc.poly([(cx - a1, cy + a1, top), (cx + a1, cy + a1, top), apex], tint, M_STONE, key + 0.001)
        sc.poly([(cx + a1, cy - a1, top), (cx + a1, cy + a1, top), apex], tint, M_STONE, key + 0.001)
        # engraving band
        for i, zz in enumerate((0.42, 0.34)):
            zz = z0 + h * zz
            ww = a0 * (0.75 - 0.15 * i) * (1 - (zz - z0) / h * 0.35)
            sc.poly([(cx + a0 * 0.97 + 0.012, cy - ww, zz), (cx + a0 * 0.97 + 0.012, cy + ww, zz),
                     (cx + a0 * 0.97 + 0.012, cy + ww, zz + 0.05), (cx + a0 * 0.97 + 0.012, cy - ww, zz + 0.05)],
                    tint * 0.55, M_ENGRAVE, key + 0.002)
        return
    if g.shape == "cross":
        bw = min(w, 0.9)
        sc.box(x0, x1, yc - bw * 0.16, yc + bw * 0.16, z0, z0 + h, tint, M_STONE, key)
        zb = z0 + h * 0.68
        sc.box(x0 + 0.02, x1 - 0.02, yc - bw * 0.5, yc + bw * 0.5, zb - 0.13, zb + 0.13, tint, M_STONE, key + 0.001)
        return
    if g.shape == "slab":
        sc.box(x0, x1, y0, y1, z0, z0 + h, tint, M_STONE, key)
        hs = h
        front_top = None
    else:
        # rounded (segmental) or gothic top
        a = w / 2
        if g.shape == "gothic":
            rise = min(a * 1.3, h * 0.45)
        else:
            rise = min(a, 0.55, h * 0.4)
        hs = z0 + h - rise
        pts = []
        nseg = 14
        if g.shape == "gothic":
            # lancet: two arcs centred on the shoulder line, meeting at the apex
            R = (rise * rise + a * a) / (2 * a)
            cy_r = yc - (R - a)
            phi = math.acos(min((R - a) / R, 1.0))
            m = nseg // 2
            right = [(cy_r + R * math.cos(phi * i / m), hs + R * math.sin(phi * i / m)) for i in range(m + 1)]
            left = [(2 * yc - py, pz) for py, pz in right[::-1][1:]]
            pts = right + left
        else:
            R = (a * a + rise * rise) / (2 * rise)
            zc = hs + rise - R
            th0 = math.acos(min(a / R, 1.0))
            for i in range(nseg + 1):
                th = th0 + (math.pi - 2 * th0) * i / nseg
                pts.append((yc + R * math.cos(th), zc + R * math.sin(th)))
        # top strips (curved surface), side, front
        for i in range(len(pts) - 1):
            (ya, za), (yb, zb) = pts[i], pts[i + 1]
            ny, nz = (zb - za), -(yb - ya)
            if ny + nz <= 0:   # facing away from the viewer
                continue
            sc.poly([(x0, ya, za), (x1, ya, za), (x1, yb, zb), (x0, yb, zb)], tint, M_STONE, key)
        sc.poly([(x0, y1, z0), (x1, y1, z0), (x1, y1, hs), (x0, y1, hs)], tint, M_STONE, key)
        front = [(x1, y0, z0), (x1, y1, z0)] + [(x1, py, pz) for py, pz in pts]
        sc.poly(front, tint, M_STONE, key + 0.0005)
    # engraving lines on the front face
    lines = [(0.62, 0.55), (0.5, 0.75), (0.41, 0.5)] if hs - z0 > 0.7 else [(0.55, 0.6)]
    for zf, wf in lines:
        zz = z0 + (hs - z0) * zf
        ww = (y1 - y0) * wf * 0.5
        sc.poly([(x1 + 0.01, yc - ww, zz), (x1 + 0.01, yc + ww, zz), (x1 + 0.01, yc + ww, zz + 0.055),
                 (x1 + 0.01, yc - ww, zz + 0.055)], tint * 0.5, M_ENGRAVE, key + 0.001)


def plot_geom(sc, g, x_off, alpha=1.0):
    if alpha <= 0.01:
        return
    xc = g.x + x_off
    x1 = xc + 0.3
    sc.poly([(x1, g.y0 + 0.05, 0), (x1 + 1.75, g.y0 + 0.05, 0), (x1 + 1.75, g.y1 - 0.05, 0), (x1, g.y1 - 0.05, 0)],
            (0, 0, 0), M_PLOT, -5e5 + xc + g.y0, aux=alpha)


def mausoleum_geom(sc, x0, x1, y0, y1):
    col = np.array((118, 138, 134), np.float32)   # system unit: green-grey marble
    key = (x0 + x1) / 2 + (y0 + y1) / 2
    sc.box(x0 - 0.3, x1 + 0.3, y0 - 0.3, y1 + 0.3, 0, 0.22, col * 0.8, M_MAUS, key - 0.3)
    sc.box(x0 - 0.15, x1 + 0.15, y0 - 0.15, y1 + 0.15, 0.22, 0.42, col * 0.88, M_MAUS, key - 0.2)
    wall_x1 = x1 - 0.9
    sc.box(x0, wall_x1, y0 + 0.15, y1 - 0.15, 0.42, 2.6, col * 0.95, M_MAUS, key - 0.1)
    # columns in the portico
    ym = (y0 + y1) / 2
    for yy in np.linspace(y0 + 0.35, y1 - 0.35, 4):
        sc.box(x1 - 0.42, x1 - 0.18, yy - 0.12, yy + 0.12, 0.42, 2.6, col, M_MAUS, x1 - 0.3 + yy)
    # door on the wall front
    sc.poly([(wall_x1 + 0.01, ym - 0.45, 0.42), (wall_x1 + 0.01, ym + 0.45, 0.42), (wall_x1 + 0.01, ym + 0.45, 1.85),
             (wall_x1 + 0.01, ym - 0.45, 1.85)], (20, 22, 30), M_DOOR, key - 0.05)
    # entablature
    sc.box(x0 - 0.12, x1 + 0.05, y0 - 0.02, y1 + 0.02, 2.6, 3.05, col * 1.02, M_MAUS, x1 + ym + 0.5)
    # pediment: gable facing +x, ridge along x
    zr, zt = 3.05, 3.85
    k2 = x1 + ym + 0.6
    sc.poly([(x0 - 0.12, y1 + 0.02, zr), (x1 + 0.05, y1 + 0.02, zr), (x1 + 0.05, ym, zt), (x0 - 0.12, ym, zt)],
            col * 0.9, M_MAUS, k2)
    sc.poly([(x1 + 0.05, y0 - 0.02, zr), (x1 + 0.05, y1 + 0.02, zr), (x1 + 0.05, ym, zt)], col, M_MAUS, k2 + 0.01)
    sc.poly([(x0 - 0.12, y0 - 0.02, zr), (x1 + 0.05, y0 - 0.02, zr), (x1 + 0.05, ym, zt), (x0 - 0.12, ym, zt)],
            col * 0.9, M_MAUS, k2 - 0.01)
    # small urn on top
    sc.box(x1 - 0.05, x1 + 0.12, ym - 0.08, ym + 0.08, zt - 0.02, zt + 0.28, col, M_MAUS, k2 + 0.02)
    return (x1 + 0.05, ym, 2.83)


def pit_geom(sc, xa, xb, ya, yb, depth, fill=0.0):
    """Open grave. fill 0..1 raises the floor (being filled in)."""
    key = -4e5 + xa + ya
    zf = -depth * (1 - fill)
    sc.poly([(xa, ya, zf), (xb, ya, zf), (xb, yb, zf), (xa, yb, zf)], (0, 0, 0), M_PIT, key, aux=0.0)
    if zf < -0.01:
        sc.poly([(xa, ya, zf), (xa, yb, zf), (xa, yb, 0), (xa, ya, 0)], (0, 0, 0), M_PIT, key + 0.1, aux=1.0)
        sc.poly([(xa, ya, zf), (xb, ya, zf), (xb, ya, 0), (xa, ya, 0)], (0, 0, 0), M_PIT, key + 0.1, aux=0.6)


def dirt_mound(sc, xa, xb, ya, yb, hgt):
    if hgt < 0.02:
        return
    key = xa + xb / 2 + ya
    xm, ym = (xa + xb) / 2, (ya + yb) / 2
    col = (86, 70, 58)
    ridge = [(xa + 0.25, ym, hgt), (xb - 0.25, ym, hgt)]
    sc.poly([(xa, yb, 0), (xb, yb, 0), ridge[1], ridge[0]], col, M_DIRT, key)
    sc.poly([(xb, ya, 0), (xb, yb, 0), ridge[1]], col, M_DIRT, key + 0.01)


def ground_occluder(sc, xa, xb, ya, yb, key_base):
    """Re-draw the ground in front of a pit so the figure inside is clipped."""
    k = key_base + 0.5
    sc.poly([(xb, ya - 0.5, 0), (xb + 2.5, ya - 0.5, 0), (xb + 2.5, yb + 2.5, 0), (xb, yb + 2.5, 0)], (0, 0, 0),
            M_GROUND, k)
    sc.poly([(xa - 0.5, yb, 0), (xb, yb, 0), (xb, yb + 2.5, 0), (xa - 0.5, yb + 2.5, 0)], (0, 0, 0), M_GROUND, k)


def zombie_geom(sc, xc, yc, zbase, key, sway=0.0, k=1.3):
    col = np.array(S.ZOMBIE, np.float32)
    dark = col * 0.78
    # legs
    sc.box(xc - 0.13 * k, xc + 0.13 * k, yc - 0.24 * k, yc - 0.03 * k, zbase, zbase + 0.75 * k, dark, M_ZOMBIE, key)
    sc.box(xc - 0.13 * k, xc + 0.13 * k, yc + 0.03 * k, yc + 0.24 * k, zbase, zbase + 0.75 * k, dark, M_ZOMBIE,
           key + 0.001)
    # torso
    zt = zbase + 0.75 * k
    sc.box(xc - 0.17 * k, xc + 0.17 * k, yc - 0.3 * k, yc + 0.3 * k, zt, zt + 0.72 * k, col, M_ZOMBIE, key + 0.002)
    # arms reaching forward (+x)
    za = zt + 0.48 * k + sway
    for side in (-1, 1):
        ya0 = yc + side * 0.3 * k
        ya1 = ya0 + side * 0.17 * k
        sc.box(xc - 0.1 * k, xc + 0.78 * k, min(ya0, ya1), max(ya0, ya1), za + (0.04 if side > 0 else -0.03) * k,
               za + 0.17 * k + (0.04 if side > 0 else -0.03) * k, col * 0.95, M_ZOMBIE, key + 0.004 + side * 0.0005)
    # head, tilted forward a touch
    zh = zt + 0.72 * k
    sc.box(xc - 0.15 * k, xc + 0.23 * k, yc - 0.18 * k, yc + 0.18 * k, zh, zh + 0.38 * k, col * 1.05, M_ZOMBIE,
           key + 0.005)
    return (xc, yc, zh + 0.38 * k)


def marker_geom(sc, x, y, kind_col):
    sc.box(x - 0.1, x + 0.1, y - 0.16, y + 0.16, 0, 0.28, kind_col, M_MARKER, x + y)


def fence_geom(sc):
    col = (26, 26, 34)
    xf, yf = X - 0.35, Y - 0.35
    posts = []
    for yy in np.arange(0.4, yf, 0.62):
        if abs(yy - (PATH_Y[0] + PATH_Y[1]) / 2) < 0.9:
            continue
        posts.append((xf, yy))
    for xx in np.arange(0.4, xf, 0.62):
        posts.append((xx, yf))
    for (px, py) in posts:
        sc.box(px - 0.04, px + 0.04, py - 0.04, py + 0.04, 0, 1.0, col, M_IRON, px + py + 0.2)
        sc.poly([(px - 0.04, py + 0.04, 1.0), (px + 0.04, py + 0.04, 1.0), (px, py, 1.16)], col, M_IRON, px + py + 0.21)
        sc.poly([(px + 0.04, py - 0.04, 1.0), (px + 0.04, py + 0.04, 1.0), (px, py, 1.16)], col, M_IRON, px + py + 0.21)
    # rails in short segments so they sort with their neighbours
    for yy in np.arange(0.4, yf - 0.62, 0.62):
        if abs(yy + 0.31 - (PATH_Y[0] + PATH_Y[1]) / 2) < 1.1:
            continue
        for zr in (0.22, 0.86):
            sc.box(xf - 0.025, xf + 0.025, yy, yy + 0.62, zr, zr + 0.05, col, M_IRON, xf + yy + 0.5)
    for xx in np.arange(0.4, xf - 0.62, 0.62):
        for zr in (0.22, 0.86):
            sc.box(xx, xx + 0.62, yf - 0.025, yf + 0.025, zr, zr + 0.05, col, M_IRON, xx + yf + 0.5)
    # gate pillars at the path
    pc = (110, 116, 128)
    for yy in (PATH_Y[0] - 0.25, PATH_Y[1] + 0.25):
        sc.box(xf - 0.2, xf + 0.2, yy - 0.2, yy + 0.2, 0, 1.45, pc, M_STONE, xf + yy + 0.3)
        sc.box(xf - 0.25, xf + 0.25, yy - 0.25, yy + 0.25, 1.45, 1.6, pc, M_STONE, xf + yy + 0.31)


def oak_polys(sc, rng, base, height, key):
    """Bare oak (decorative), drawn as thin quads in the plane facing the viewer."""
    bx, by = base
    perp = np.array([1.0, -1.0, 0.0]) / math.sqrt(2)  # screen-horizontal direction in world

    def branch(p, ang, length, width, depth):
        if depth == 0 or length < 0.15:
            return
        d = np.array([math.sin(ang), math.cos(ang)])
        q = p + d * length
        a3 = np.array([bx, by, 0]) + perp * p[0] + np.array([0, 0, p[1]])
        b3 = np.array([bx, by, 0]) + perp * q[0] + np.array([0, 0, q[1]])
        n = np.array([d[1], -d[0]])
        w0, w1 = width, width * 0.68
        pa = [a3 + perp * n[0] * w0 + np.array([0, 0, n[1] * w0]), a3 - perp * n[0] * w0 - np.array([0, 0, n[1] * w0])]
        pb = [b3 - perp * n[0] * w1 - np.array([0, 0, n[1] * w1]), b3 + perp * n[0] * w1 + np.array([0, 0, n[1] * w1])]
        sc.poly([pa[0], pa[1], pb[0], pb[1]], (0, 0, 0), M_TREE, key)
        k = 2 if depth > 2 else int(rng.integers(1, 3))
        for i in range(k):
            na = ang + rng.uniform(-0.9, 0.9) + (0.35 if i == 0 else -0.35)
            branch(q, na, length * rng.uniform(0.6, 0.82), w1, depth - 1)

    trunk_h = height * 0.32
    branch(np.array([0.0, 0.0]), rng.uniform(-0.12, 0.12), trunk_h, 0.32, 1)
    top = np.array([0.0, trunk_h])
    for i in range(4):
        branch(top, -1.25 + i * 0.8 + rng.uniform(-0.2, 0.2), height * 0.3, 0.2, 6)


# ----------------------------------------------------------------------------- rendering
FOG_NEAR = np.array((44, 48, 78), np.float32)
FOG_FAR = np.array((112, 108, 150), np.float32)


class Look:
    pass


def build_textures():
    L = Look()
    L.grass = E.noise_tex(512, [2, 6, 18], 11)
    L.speck = E.noise_tex(256, [0.7, 2.5], 12, [1.0, 0.6])
    L.wisp = E.noise_tex(512, [10, 28, 60], 13, [0.5, 1.0, 0.8])
    L.soil = E.noise_tex(256, [1.2, 5], 14)
    return L


def sky_canvas(w, h, moon_xy, moon_r):
    c = S.sky(w, h, glow=(150, 66, 150), base=(8, 8, 20), centre=(0.86, 0.10))
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    # haze where the cemetery recedes (behind the back corner, top of frame)
    band = np.exp(-(((yy + h * 0.05) / (h * 0.42)) ** 2)) * np.exp(-(((xx - w * 0.42) / (w * 0.36)) ** 2))
    c += band[..., None] * np.array((70, 66, 102), np.float32)
    haze = c.copy()
    # stars, sparse and faint, hidden near the fog bank
    rng = np.random.default_rng(5)
    n = int(w * h / 9000)
    sx, sy = rng.uniform(0, w, n), rng.uniform(0, h * 0.55, n)
    for x, y in zip(sx, sy):
        b = rng.uniform(0.25, 1.0) * (1 - band[int(y), int(x)] * 0.9)
        S.glow(c, x, y, 0.6 * w / 1600 + 0.3, (190, 196, 230), strength=0.55 * b)
    draw_moon(c, moon_xy[0], moon_xy[1], moon_r, moon_age(DEMO_TIME))
    return c, haze


def draw_moon(c, mx, my, R, age):
    theta = 2 * math.pi * age / SYNODIC
    light = np.array([math.sin(theta), 0.0, -math.cos(theta)])   # x right, y up, z towards viewer
    # halo first
    S.glow(c, mx, my, R * 3.2, (50, 52, 84), strength=0.45)
    h, w, _ = c.shape
    r = int(R + 3)
    x0, x1, y0, y1 = int(mx) - r, int(mx) + r + 1, int(my) - r, int(my) + r + 1
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    dx, dy = (xx + 0.5 - mx) / R, -(yy + 0.5 - my) / R
    d2 = dx * dx + dy * dy
    dz = np.sqrt(np.clip(1 - d2, 0, 1))
    lam = dx * light[0] + dy * light[1] + dz * light[2]
    lit = E.smoothstep(-0.09, 0.04, lam)
    tex = E.noise_tex(128, [3, 9], 21)
    ty = np.clip(((dy * 0.5 + 0.5) * 127), 0, 127).astype(int)
    tx = np.clip(((dx * 0.5 + 0.5) * 127), 0, 127).astype(int)
    maria = 0.78 + 0.3 * tex[ty, tx]
    lit_col = np.array((240, 236, 222), np.float32)
    earth = np.array((58, 60, 92), np.float32)   # earthshine
    col = lit[..., None] * lit_col * maria[..., None] + (1 - lit[..., None]) * earth * maria[..., None]
    edge = np.clip((1 - np.sqrt(d2)) * R, 0, 1)[..., None]
    reg = c[y0:y1, x0:x1]
    reg[:] = reg * (1 - edge) + col * edge
    # the thin lit limb, traced with small glows so a 2% crescent still reads at this size
    side = -1.0 if light[0] < 0 else 1.0
    for a in np.linspace(-math.pi / 2 * 0.9, math.pi / 2 * 0.9, 90):
        wgt = math.cos(a) ** 2.2
        rr = R * (0.99 - 0.03 * wgt)
        S.glow(c, mx + side * rr * math.cos(a), my + rr * math.sin(a), max(0.8, R * (0.02 + 0.03 * wgt)),
               (255, 248, 226), strength=0.05 + 0.75 * wgt)
    S.glow(c, mx + side * R * 0.9, my, R * 0.5, (110, 106, 120), strength=0.35)


def shade(g, iso, L, t, slide, fog_density, skies):
    """Per-pixel shading, fog and compositing. Returns float canvas."""
    H, W = g.ids.shape
    sky, haze = skies
    x, y, z = g.world[..., 0], g.world[..., 1], g.world[..., 2]
    n = g.normal
    out = np.zeros((H, W, 3), np.float32)
    mat = g.mat
    # content coordinates (graves slide back when a new row is born)
    xc = x + slide
    # --- ground: grass, paths, plots
    gtex = E.sample(L.grass, xc * 14.0, y * 14.0)
    grass = np.array((28, 44, 44), np.float32) * (0.68 + 0.62 * gtex[..., None])
    rowpos = (xc - ROW0_X) / ROW_DX
    frac = rowpos - np.floor(rowpos)
    path_mask = E.smoothstep(0.63, 0.67, frac) * (1 - E.smoothstep(0.86, 0.9, frac))
    cross = E.smoothstep(PATH_Y[0] - 0.05, PATH_Y[0] + 0.1, y) * (1 - E.smoothstep(PATH_Y[1] - 0.1, PATH_Y[1] + 0.05, y))
    front = E.smoothstep(ROW0_X + 2.6, ROW0_X + 2.7, xc)
    path_mask = np.maximum(path_mask * (1 - front), cross * (xc > 0.5))
    stex = E.sample(L.speck, xc * 40.0, y * 40.0)
    gravel = np.array((66, 66, 82), np.float32) * (0.75 + 0.45 * stex[..., None])
    ground = grass * (1 - path_mask[..., None]) + gravel * path_mask[..., None]
    gm = (mat == M_GROUND)
    out[gm] = ground[gm]
    pm = (mat == M_PLOT)
    plot = np.array((24, 38, 34), np.float32) * (0.75 + 0.5 * gtex[..., None])
    a = g.aux[..., None]
    out[pm] = (plot * a + ground * (1 - a))[pm]
    # --- stone: soft sky light from above + faint moon rim, lichen speckle, damp feet
    up = np.clip(n[..., 2], 0, 1)
    face = 0.5 + 0.55 * up + 0.2 * np.clip(n[..., 0], 0, 1)
    spk = E.sample(L.speck, (y + x * 0.7) * 30.0, (z + xc * 0.3) * 30.0)
    damp = 0.62 + 0.38 * E.smoothstep(0.0, 0.9, z)
    sm = np.isin(mat, (M_STONE, M_MAUS))
    stone = g.color * face[..., None] * (0.82 + 0.3 * spk[..., None]) * damp[..., None]
    moss = np.array((60, 84, 60), np.float32)
    mossy = (1 - E.smoothstep(0.0, 0.35, z)) * 0.35 * spk
    stone = stone * (1 - mossy[..., None]) + moss * mossy[..., None]
    stone += (up ** 3 * 22.0)[..., None] * np.array((0.8, 0.9, 1.15), np.float32)   # cool moonlit tops
    out[sm] = stone[sm]
    em = (mat == M_ENGRAVE)
    out[em] = (g.color * 0.62)[em]
    # --- soil sides, pits, dirt
    so = (mat == M_SOIL)
    strata = 0.75 + 0.25 * np.sin(z * 9.0 + E.sample(L.soil, (x + y) * 6.0, z * 6.0) * 3.0)
    soil = np.array((58, 44, 40), np.float32) * strata[..., None] * (0.7 + 0.15 * (n[..., 0] > 0.5))[..., None]
    out[so] = soil[so]
    pit = (mat == M_PIT)
    out[pit] = (np.array((30, 24, 24), np.float32) * (0.6 + 0.4 * g.aux[..., None]) * (0.85 + 0.3 * stex[..., None]))[pit]
    dm = (mat == M_DIRT)
    out[dm] = (np.array((80, 64, 52), np.float32) * (0.6 + 0.5 * up[..., None]) * (0.8 + 0.4 * stex[..., None]))[dm]
    # --- zombie: lit from within a little
    zm = (mat == M_ZOMBIE)
    out[zm] = (g.color * (0.62 + 0.38 * up[..., None] + 0.18 * np.clip(n[..., 0:1], 0, 1)))[zm]
    mk = (mat == M_MARKER)
    out[mk] = (g.color * (0.6 + 0.4 * up[..., None]))[mk]
    ir = (mat == M_IRON)
    out[ir] = (g.color * (0.9 + 0.6 * up[..., None]))[ir]
    dr = (mat == M_DOOR)
    out[dr] = g.color[dr]
    tr = (mat == M_TREE)
    out[tr] = np.array((26, 24, 34), np.float32)
    # --- ambient occlusion around stones (precomputed mask)
    if hasattr(g, "ao"):
        groundish = np.isin(mat, (M_GROUND, M_PLOT))
        out[groundish] *= (1 - 0.45 * g.ao[groundish])[..., None]
    # --- fog: distance + low ground fog modulated by drifting wisps
    far = (X + Y) - (x + y)
    dist_f = 1 - np.exp(-((np.clip(far - 10.0, 0, None) / 23.0) ** 2) * fog_density)
    wa = (x - y) * 4.0 + t * 1.6
    wb = (x + y) * 9.0 + t * 0.5
    wisp = E.sample(L.wisp, wa, wb)
    nearness = 0.35 + 0.65 * np.clip(far / 30.0, 0, 1)
    low = np.exp(-np.clip(z, 0, None) / 0.6) * (0.05 + 0.55 * E.smoothstep(0.42, 0.8, wisp)) * fog_density * nearness
    fog = 1 - (1 - dist_f) * (1 - np.clip(low, 0, 0.9))
    fog = np.where(mat == M_TREE, np.maximum(fog, 0.45), fog)
    tcol = E.smoothstep(14.0, 44.0, far)[..., None]
    fogcol = FOG_NEAR * (1 - tcol) + haze * 1.08 * tcol
    out = out * (1 - fog[..., None]) + fogcol * fog[..., None]
    # sky behind
    out = np.where(g.valid[..., None], out, sky)
    # floating fog layers in front of everything (screen-space planes z = h)
    for hgt, strength, sc_ in ((0.35, 0.32, 1.0), (1.2, 0.2, 0.7)):
        vv = g.v + hgt
        xw = (g.u + 2 * vv) / 2
        yw = (2 * vv - g.u) / 2
        inside = E.smoothstep(-1.5, 1.5, xw) * E.smoothstep(-1.5, 1.5, yw) * \
            (1 - E.smoothstep(X - 1.0, X + 2.0, xw)) * (1 - E.smoothstep(Y - 1.0, Y + 2.0, yw))
        infront = np.where(g.valid, (xw + yw + hgt) > (x + y + z), True)
        wn = E.sample(L.wisp, (xw - yw) * 3.0 * sc_ + t * 2.2 + hgt * 50, (xw + yw) * 7.0 * sc_ + t * 0.4)
        dens = E.smoothstep(0.5, 0.85, wn) * strength * inside * infront * min(fog_density, 1.4)
        farw = E.smoothstep(14.0, 44.0, (X + Y) - (xw + yw))[..., None]
        wc = FOG_NEAR * 1.6 * (1 - farw) + haze * 1.15 * farw
        out = out * (1 - dens[..., None]) + wc * dens[..., None]
    return out


def ao_mask(iso, graves, x_off_fn, extra_boxes):
    img = Image.new("L", (iso.w, iso.h), 0)
    d = ImageDraw.Draw(img)
    for g, xo in graves:
        xc = g.x + xo
        th = 0.5 if g.shape != "mass" else 2.0
        pts = iso.proj(np.array([(xc - th, g.y0 - 0.3, 0), (xc + th + 0.4, g.y0 - 0.3, 0),
                                 (xc + th + 0.4, g.y1 + 0.35, 0), (xc - th, g.y1 + 0.35, 0)]))
        d.polygon([tuple(p) for p in pts], fill=255)
    for (x0, x1, y0, y1) in extra_boxes:
        pts = iso.proj(np.array([(x0, y0, 0), (x1, y0, 0), (x1, y1, 0), (x0, y1, 0)]))
        d.polygon([tuple(p) for p in pts], fill=255)
    img = img.filter(ImageFilter.GaussianBlur(iso.s * 0.35))
    return np.asarray(img, np.float32) / 255.0


# ----------------------------------------------------------------------------- scene state
class World:
    def __init__(self):
        rng = np.random.default_rng(1977)
        self.rows = make_rows(rng)
        self.chrome = special_graves(self.rows)
        self.new_row = burst_row(np.random.default_rng(4))
        rng2 = np.random.default_rng(31)
        self.markers = []
        for i in range(23):
            self.markers.append((26.9 + (i % 4) * 0.62 + rng2.uniform(-0.08, 0.08),
                                 1.4 + (i // 4) * 0.75 + rng2.uniform(-0.1, 0.1),
                                 ["session", "session", "system", "container"][int(rng2.integers(4))]))
        self.maus = (ROW0_X - 2 * ROW_DX - 0.3, ROW0_X - ROW_DX + 0.5, MAUS_Y[0], MAUS_Y[1])
        # ghost source: an old grave in row 5
        self.ghost_grave = sorted(self.rows[3], key=lambda g: abs((g.y0 + g.y1) / 2 - 15.5))[0]
        self.ghost_grave.name, self.ghost_grave.pid = "sshd-4127", 4127
        # zombies in open graves at the front
        self.pits = [(ROW0_X + 2.85, ROW0_X + 5.0, 16.4, 17.55), (ROW0_X + 2.85, ROW0_X + 5.0, 20.0, 21.15)]
        self.zombie_names = ["pipewire-2231", "worker-2290"]


def scene_for(world, iso, t, phase):
    """phase: dict of animation parameters.
    slide: 0..1 rows moving back; new_rise: per-stone rise list; reap: 0..1 for zombie 1"""
    sc = E.Scene()
    slide = phase.get("slide", 0.0) * ROW_DX
    sc.poly([(0, 0, 0), (X, 0, 0), (X, Y, 0), (0, Y, 0)], (0, 0, 0), M_GROUND, -1e6)
    T = 1.3
    sc.poly([(X, 0, -T), (X, Y, -T), (X, Y, 0), (X, 0, 0)], (0, 0, 0), M_SOIL, -9e5)
    sc.poly([(0, Y, -T), (X, Y, -T), (X, Y, 0), (0, Y, 0)], (0, 0, 0), M_SOIL, -9e5)
    graves = []
    have_new = phase.get("have_new", False)
    for r, row in enumerate(world.rows):
        base_x = ROW0_X - (r + (1 if have_new else 0)) * ROW_DX
        x_off = -slide if not have_new else 0.0
        if have_new:
            x_off = -(phase.get("slide", 1.0) - 1.0) * ROW_DX
        for g in row:
            g.x = base_x
            xx = g.x + x_off
            sink = 0.0
            zsc = 1.0
            if xx < 1.2:
                zsc = max(0.0, (xx - (-1.0)) / 2.2)
            if xx < -0.9:
                continue
            graves.append((g, x_off))
            plot_geom(sc, g, x_off, alpha=min(1.0, zsc))
            stone_geom(sc, g, x_off, z_scale=zsc)
    if have_new:
        rises = phase.get("rises", [1.0] * len(world.new_row))
        x_off = -(phase.get("slide", 1.0) - 1.0) * ROW_DX
        for g, rz in zip(world.new_row, rises):
            g.x = ROW0_X
            if rz <= 0:
                continue
            plot_geom(sc, g, x_off, alpha=min(1.0, rz * 2))
            stone_geom(sc, g, x_off, z_scale=rz)
            graves.append((g, x_off))
    # mausoleum (moves with its row)
    mx0, mx1, my0, my1 = world.maus
    m_off = (-slide if not have_new else -(phase.get("slide", 1.0) - 1.0) * ROW_DX) - (ROW_DX if have_new else 0)
    maus_anchor = mausoleum_geom(sc, mx0 + m_off, mx1 + m_off, my0, my1)
    # potter's field: unmarked markers
    for (mxp, myp, kind) in world.markers:
        marker_geom(sc, mxp, myp, np.array(STONE_TINT[kind], np.float32) * 0.8)
    # open graves + zombies
    zombie_tops = []
    reap = phase.get("reap", 0.0)
    for i, (xa, xb, ya, yb) in enumerate(world.pits):
        fill = 0.0
        if i == 0 and reap > 0:
            fill = E.smoothstep(0.25, 0.7, reap)
        if fill < 0.999:
            pit_geom(sc, xa, xb, ya, yb, 1.1, fill)
        sink = 0.0
        alive = True
        if i == 0:
            sink = E.smoothstep(0.0, 0.45, reap) * 2.2
            alive = reap < 0.45
        key = -3e5 + xa + ya
        if alive:
            top = zombie_geom(sc, (xa + xb) / 2 - 0.3, (ya + yb) / 2, -1.1 - sink, key,
                              sway=0.05 * math.sin(t * 2.0 + i * 1.7))
            zombie_tops.append(((xa + xb) / 2 - 0.3, (ya + yb) / 2, top[2], i))
        ground_occluder(sc, xa, xb, ya, yb, key)
        mound_h = 0.42 * (1 - (E.smoothstep(0.25, 0.7, reap) if i == 0 else 0.0))
        dirt_mound(sc, xa + 0.1, xb - 0.1, yb + 0.15, yb + 1.0, mound_h * 1.2)
    # reaped zombie gets a headstone
    reaped = None
    if reap > 0.55:
        xa, xb, ya, yb = world.pits[0]
        g = Grave(world.zombie_names[0], 2231, "session", 8.4e4, 210, "reaped by wireplumber", 0, "round")
        g.x, g.y0, g.y1 = xa - 0.25, ya - 0.2, yb + 0.2
        g.kind = "session"
        rz = E.smoothstep(0.55, 0.95, reap)
        stone_geom(sc, g, 0.0, z_scale=rz)
        reaped = g
        graves.append((g, 0.0))
    fence_geom(sc)
    # decorative bare oaks deep in the fog
    rng = np.random.default_rng(8)
    oak_polys(sc, rng, (1.4, 17.5), 8.5, -1e5 + 1)
    g = E.raster(sc, iso)
    g.ao = ao_mask(iso, graves, None, [(mx0 + m_off - 0.5, mx1 + m_off + 0.5, my0 - 0.5, my1 + 0.5)])
    info = dict(graves=graves, maus=(maus_anchor[0] + m_off, maus_anchor[1], maus_anchor[2]),
                zombies=zombie_tops, reaped=reaped, slide=slide if not have_new else
                (phase.get("slide", 1.0) - 1.0) * ROW_DX)
    return g, info


# ----------------------------------------------------------------------------- overlays
def ghost(canvas, cx, cy, size, alpha, t, base_y=None):
    """Translucent wraith in final-res pixels: hooded head, robe trailing to a wisp that
    reaches back down to its grave."""
    if alpha <= 0:
        return
    H, W, _ = canvas.shape
    if base_y is not None and base_y > cy + size * 0.9:
        steps = 24
        span = min(1.0, (base_y - cy - size * 0.6) / (size * 1.2))
        for i in range(steps):
            f = i / (steps - 1)
            yy = cy + size * 0.6 + (base_y - cy - size * 0.6) * f
            xx = cx + math.sin(f * 5 + t * 3) * size * 0.06 * f
            S.glow(canvas, xx, yy, size * 0.07 * (1 - 0.5 * f), (110, 170, 200), strength=0.4 * alpha * (1 - f) * span)
    S.glow(canvas, cx, cy - size * 0.1, size * 0.75, (60, 110, 140), strength=0.55 * alpha)
    m = Image.new("L", (W, H), 0)
    d = ImageDraw.Draw(m)
    hw = size * 0.2
    top = cy - size * 0.62
    head = [(cx + hw * math.cos(a), top + hw - hw * 1.15 * math.sin(a))
            for a in np.linspace(0, math.pi, 20)]
    left = []
    right = []
    for i in range(1, 13):
        f = i / 12
        yy = top + hw + size * 1.15 * f
        width = hw * (1.0 + 0.9 * math.sin(min(f, 0.75) / 0.75 * math.pi / 2)) * (1 - 0.75 * max(0, f - 0.6) / 0.4)
        wave = math.sin(f * 7 + t * 3.0) * size * 0.05 * f
        left.append((cx - width + wave, yy))
        right.append((cx + width + wave, yy))
    tip = (cx + math.sin(t * 3 + 7) * size * 0.08, top + hw + size * 1.25)
    poly = [right[0]] + head[::-1] + [left[0]] if False else head + left + [tip] + right[::-1]
    d.polygon(poly, fill=255)
    m = m.filter(ImageFilter.GaussianBlur(max(1.0, size * 0.035)))
    a = np.asarray(m, np.float32) / 255.0
    yy = np.arange(H, dtype=np.float32)[:, None]
    fade = np.clip(1 - (yy - (cy - size * 0.2)) / (size * 0.95), 0.0, 1.0)
    a = (a * (0.25 + 0.5 * fade))[..., None] * alpha
    col = np.array((210, 236, 246), np.float32)
    canvas[:] = canvas * (1 - a) + col * a
    S.glow(canvas, cx, top + hw * 1.1, size * 0.22, (120, 170, 190), strength=0.45 * alpha)
    for ex in (-0.38, 0.38):
        S.glow(canvas, cx + ex * hw, top + hw * 1.15, size * 0.045, (-90, -70, -60), strength=alpha)


def callout(draw, img_w, anchor, box_xy, lines, colour, sz):
    f1 = S.font(sz, S.FONT_MONO_BOLD)
    f2 = S.font(sz)
    pad = int(sz * 0.6)
    widths = [draw.textlength(lines[0], font=f1)] + [draw.textlength(s, font=f2) for s in lines[1:]]
    bw = int(max(widths)) + pad * 2
    bh = int(sz * 1.35 * len(lines)) + pad * 2 - int(sz * 0.3)
    bx, by = box_xy
    bx = min(max(bx, 6), img_w - bw - 6)
    ax, ay = anchor
    # leader line to the nearest box edge
    lx = bx if ax < bx else (bx + bw if ax > bx + bw else ax)
    ly = by + bh if ay > by + bh else (by if ay < by else ay)
    draw.line([(ax, ay), (lx, ly)], fill=colour + (200,), width=1)
    draw.ellipse([ax - 2.5, ay - 2.5, ax + 2.5, ay + 2.5], fill=colour + (255,))
    draw.rounded_rectangle([bx, by, bx + bw, by + bh], radius=4, fill=(14, 14, 22, 228), outline=colour + (230,))
    draw.text((bx + pad, by + pad), lines[0], font=f1, fill=(232, 234, 242, 255))
    for i, s in enumerate(lines[1:]):
        draw.text((bx + pad, by + pad + int(sz * 1.35 * (i + 1))), s, font=f2, fill=(176, 182, 200, 255))


def label_soft(draw, x, y, text, size, fill=(200, 204, 218), anchor="mm"):
    fill = tuple(fill) if len(fill) == 4 else tuple(fill) + (255,)
    if fill[3] <= 0:
        return
    f = S.font(size)
    draw.text((x + 1, y + 1), text, font=f, fill=(0, 0, 0, fill[3]), anchor=anchor)
    draw.text((x, y), text, font=f, fill=fill, anchor=anchor)


def engrave_text(img, iso_final, x_face, yc, z, text, size_px, colour):
    """Draw text on a +x face, reading right-up along -y (sheared into the face plane)."""
    f = S.font(size_px, S.FONT_SERIF_BOLD)
    tw = int(f.getlength(text)) + 4
    th = size_px + 6
    t_img = Image.new("L", (tw, th), 0)
    ImageDraw.Draw(t_img).text((2, 2), text, font=f, fill=255)
    s = iso_final.s
    # text pixel (u, v) -> world: y = yc + (tw/2 - u)/k, z = z + (th/2 - v)/k; k = pixels per world unit in text
    k = size_px / 0.32
    # screen = ox + (x - y)s, oy + (x + y)s/2 - z s
    # derivatives: d screen/du = (+s/k, -s/(2k)); d screen/dv = (0, +s/k)
    sx0, sy0 = iso_final.proj(np.array([x_face, yc, z]))
    a, b = s / k, 0.0
    c_, d_ = -s / (2 * k), s / k
    # screen = M (u - tw/2, v - th/2) + (sx0, sy0); inverse for Image.transform
    M = np.array([[a, b], [c_, d_]])
    Mi = np.linalg.inv(M)
    W, H = img.size
    # output pixel (X, Y) -> input (u, v) = Mi((X, Y) - (sx0, sy0)) + (tw/2, th/2)
    coeffs = (Mi[0, 0], Mi[0, 1], tw / 2 - Mi[0, 0] * sx0 - Mi[0, 1] * sy0,
              Mi[1, 0], Mi[1, 1], th / 2 - Mi[1, 0] * sx0 - Mi[1, 1] * sy0)
    warped = t_img.transform((W, H), Image.AFFINE, coeffs, resample=Image.BILINEAR)
    layer = Image.new("RGBA", (W, H), colour + (0,))
    layer.putalpha(warped)
    img.alpha_composite(layer)


_SKY = {}


def render(world, W, H, ss, t, phase, overlays):
    band_lines = overlays["status"]
    sz = max(11, W // 115)
    band = int(sz * 1.45) * len(band_lines) + 10
    scene_h = H - band
    k = W / 1600
    s_final = 31 * k
    ox_f = 676 * k
    oy_f = scene_h - 908 * k
    SW, SH = int(round(W * ss)), int(round(scene_h * ss))
    ss = SW / W
    iso = E.Iso(SW, SH, s_final * ss, ox_f * ss, oy_f * ss)
    iso_f = E.Iso(W, scene_h, s_final, ox_f, oy_f)
    moon_xy = (1392 * k * ss, 104 * k * ss)
    key = (SW, SH)
    if key not in _SKY:
        _SKY[key] = sky_canvas(SW, SH, moon_xy, 34 * k * ss)
    sky = _SKY[key]
    g, info = scene_for(world, iso, t, phase)
    canvas = shade(g, iso, world.L, t, info["slide"], phase.get("fog", 1.0), sky)
    img = E.downsample(canvas, ss, (W, scene_h))
    c = S.to_canvas(img)
    # zombie glow
    for (zx, zy, ztop, i) in info["zombies"]:
        px, py = iso_f.proj(np.array([zx, zy, ztop * 0.45]))
        S.glow(c, px, py, 18 * W / 1600, S.ZOMBIE, strength=0.28)
    # ghost
    gp = phase.get("ghost", 0.0)
    if gp > 0:
        gg = world.ghost_grave
        px, py = iso_f.proj(np.array([phase["ghost_x"], (gg.y0 + gg.y1) / 2, 1.2 + gp * 2.2]))
        alpha = E.smoothstep(0.0, 0.25, gp) * (1 - E.smoothstep(0.85, 1.0, gp) * 0.6)
        bx, by = iso_f.proj(np.array([phase["ghost_x"], (gg.y0 + gg.y1) / 2, gg.h * 0.6]))
        ghost(c, px, py, 92 * W / 1600, float(alpha), t, base_y=by)
        overlays["ghost_label"] = (px + 34 * W / 1600, py - 30 * W / 1600, float(alpha))
    img = S.to_image(c).convert("RGBA")
    # engraved inscription on the mausoleum frieze
    mx, my, mz = info["maus"]
    engrave_text(img, iso_f, mx + 0.01, my, mz, "CUPS.SERVICE", max(8, int(13 * W / 1600)), (40, 52, 52))
    full = Image.new("RGBA", (W, H), S.PANEL + (255,))
    full.paste(img, (0, 0))
    ov = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(ov)
    overlays["draw"](d, iso_f, info, W, scene_h)
    full.alpha_composite(ov)
    full = full.convert("RGB")
    S.status(full, band_lines, size=sz)
    return full


def status_lines(graves, zombies, deaths, extra="", short=False):
    age = moon_age(DEMO_TIME)
    lit = (1 - math.cos(2 * math.pi * age / SYNODIC)) / 2
    if short:
        return [
            f"ISOTOP / NECROPOLIS / DEMO   192 processes | {graves} graves | {zombies} zombies | "
            f"{deaths} deaths/min | RAM 20.0 / 32.0 GiB",
            "row = minute of death, newest in front | height = lifetime CPU (log) | width = peak RSS | tint = kind",
            f"pink = zombie | fog = memory pressure 14% | moon: {moon_phase_name(age)} {lit * 100:.0f}% lit (real) | "
            "click a grave for its epitaph",
        ]
    return [
        f"ISOTOP / NECROPOLIS / DEMO   192 processes | {graves} graves | {zombies} zombies | 1 mausoleum | "
        f"{deaths} deaths/min | RAM 20.0 GiB / 32.0 GiB{extra}",
        "Row = minute of death, newest in front | height = lifetime CPU (log) | width = peak RSS | tint = kind | "
        "pink = zombie | fog = memory pressure 14%",
        f"deaths: proc connector + taskstats | moon: {moon_phase_name(age)} {lit * 100:.0f}% lit, age {age:.1f} d "
        f"(real) | click a grave for its epitaph | Tab next view | ? help | q quit",
    ]


def grave_anchor(iso_f, g, x_off):
    xc = g.x + x_off
    return iso_f.proj(np.array([xc + 0.2, (g.y0 + g.y1) / 2, g.h * 0.92]))


def find(world, name):
    for row in world.rows + [world.new_row]:
        for g in row:
            if g.name == name:
                return g
    return None


def epitaph(g):
    return [f"{g.name}", f"lived {human_time(g.lifetime)}   {g.cause}"]


def make_overlay(world, overlays, rows_minutes, callouts, reaped_alpha=0.0):
    """rows_minutes: list of (world x of row, minute). callouts: list of names to show."""

    def draw_ov(d, iso_f, info, W, sh):
        k = W / 1600
        sz = max(10, round(13 * k))
        lab = max(10, round(12 * k))
        for xr, minute in rows_minutes:
            if xr < 1.0:
                continue
            px, py = iso_f.proj(np.array([xr + 0.9, Y - 0.6, 0.0]))
            label_soft(d, px - 6 * k, py + 2 * k, f"04:{minute:02d}", lab, fill=(150, 156, 178), anchor="rm")
        for (zx, zy, ztop, i) in info["zombies"]:
            px, py = iso_f.proj(np.array([zx, zy, ztop + 0.25]))
            label_soft(d, px, py - 22 * k, world.zombie_names[i], lab, fill=(246, 170, 190))
            label_soft(d, px, py - 8 * k, "awaiting wait()", lab, fill=(200, 150, 170))
        if info["reaped"] is not None and reaped_alpha > 0:
            g = info["reaped"]
            px, py = grave_anchor(iso_f, g, 0)
            a = int(255 * reaped_alpha)
            label_soft(d, px + 26 * k, py + 16 * k, "pipewire-2231  reaped, exited 0", lab,
                       fill=(226, 200, 208, a), anchor="lm")
        mx, my, mz = info["maus"]
        px, py = iso_f.proj(np.array([mx - 1.5, my, 4.25]))
        if py > 14 * k:
            label_soft(d, px, py - 10 * k, "cups.service  (unit stopped, 6 graves)", lab, fill=(170, 210, 205))
        px, py = iso_f.proj(np.array([28.0, 1.4, 0.4]))
        label_soft(d, W - 10 * k, py - 22 * k, "23 died between samples", lab, fill=(160, 166, 186), anchor="rm")
        mg = [gg_ for gg_ in world.new_row if gg_.shape == "mass"][0]
        if "mass" in callouts:
            px, py = grave_anchor(iso_f, mg, -info["slide"])
            label_soft(d, px + 6 * k, py - 10 * k, "cc1 x 31  mass grave (make-77)", lab, fill=(206, 196, 176))
        if "ghost_label" in overlays:
            gx, gy, ga = overlays["ghost_label"]
            a = int(255 * min(1.0, ga * 1.4))
            label_soft(d, gx, gy - 7 * k, "pid 4127 reused", lab, fill=(190, 222, 236, a), anchor="lm")
            label_soft(d, gx, gy + 8 * k, "sshd-4127 rests below", max(9, lab - 1), fill=(140, 170, 186, a),
                       anchor="lm")
        boxes = {"gcc-4411": (430, 724), "chrome-812": (880, 210), "make-77": (1090, 436)}
        for name in callouts:
            if name not in boxes:
                continue
            g = find(world, name)
            ax, ay = grave_anchor(iso_f, g, -info["slide"])
            col = tuple(int(v) for v in np.array(S.KIND[g.kind]) * 0.9)
            bx, by = boxes[name]
            callout(d, W, (ax, ay), (bx * k, by * k), epitaph(g), col, sz)

    return draw_ov


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "still"
    fast = "--fast" in sys.argv
    world = World()
    world.L = build_textures()
    if mode in ("still", "both"):
        W, H = 1600, 900
        ss = 1 if fast else 2
        phase = dict(have_new=True, slide=1.0, rises=[1.0] * len(world.new_row), reap=0.0, ghost=0.62, fog=1.0)
        phase["ghost_x"] = ROW0_X - (3 + 1) * ROW_DX + 0.2
        overlays = dict(status=status_lines(318, 2, 41))
        rows = [(ROW0_X - r * ROW_DX, 12 - r) for r in range(5)]
        overlays["draw"] = make_overlay(world, overlays, rows, ["gcc-4411", "chrome-812", "make-77", "mass"])
        img = render(world, W, H, ss, 3.0, phase, overlays)
        path = S.save_still(img, "necropolis" if not fast else "necropolis-fast")
        print("still", path)
    if mode in ("anim", "both"):
        W, H = 960, 540
        ss = 1.0 if fast else 1.5
        fps, dur = 12, 9.0
        n = int(fps * dur)
        if "--frames" in sys.argv:
            picks = [int(v) for v in sys.argv[sys.argv.index("--frames") + 1].split(",")]
        else:
            picks = list(range(n))
        frames = []
        for fi in picks:
            t = fi / fps
            slide = float(E.smoothstep(1.0, 2.8, t))
            rises = [float(E.smoothstep(3.0 + i * 0.24, 3.9 + i * 0.24, t)) for i in range(len(world.new_row))]
            reap = float(np.clip((t - 5.7) / 2.5, 0, 1))
            gp = 0.12 + 0.82 * t / dur
            info_slide = (slide - 1.0) * ROW_DX
            phase = dict(have_new=True, slide=slide, rises=rises, reap=reap, ghost=gp, fog=1.0,
                         ghost_x=ROW0_X - 4 * ROW_DX - info_slide + 0.2)
            risen = sum(1 for r in rises if r > 0.5)
            zombies = 2 if reap < 0.45 else 1
            graves = 310 + risen + (1 if reap > 0.7 else 0)
            deaths = 9 if risen == 0 else 9 + 32 * risen // 8
            overlays = dict(status=status_lines(graves, zombies, deaths, short=True))
            rows = [(ROW0_X - (r + 1) * ROW_DX - info_slide, 11 - r) for r in range(5)]
            if rises[0] > 0.05:
                rows.append((ROW0_X - info_slide, 12))
            calls = ["chrome-812"]
            if t > 4.6:
                calls.append("make-77")
                calls.append("mass")
            if t > 5.2:
                calls.append("gcc-4411")
            overlays["draw"] = make_overlay(world, overlays, rows, calls,
                                            reaped_alpha=float(E.smoothstep(0.75, 0.95, reap)))
            frames.append(render(world, W, H, ss, t, phase, overlays))
            print("frame", fi, flush=True)
        if "--frames" in sys.argv:
            sheet = Image.new("RGB", (W * 2, H * 2))
            for j, f in enumerate(frames[:4]):
                sheet.paste(f, ((j % 2) * W, (j // 2) * H))
            sheet.save(S.OUT + "/necropolis-sheet.png")
            print("sheet")
        else:
            print(S.save_frames(frames, "necropolis", fps=fps))


if __name__ == "__main__":
    main()
