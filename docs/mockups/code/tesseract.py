"""Mockup of isotop's tesseract view: processes as worldlines in a tesseract whose fourth axis
w is the last 60 s. x = CPU-seconds since the window opened (30 CPU-s per unit, so slope = CPU and
c = one core), y = memory, z = seat. 4D rotation, then a Schlegel perspective divide along w
(eye at w = 3.5), then isotop's orthographic camera.

usage: python3 -I tesseract.py still|anim|both
"""
import itertools
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
from PIL import ImageDraw

import isostyle as S

RNG = np.random.default_rng(5)
SAMPLES = 61
EYE = 3.5
CHERENKOV = (90, 170, 255)


def make_processes():
    """Demo population: (name, kind, seat z, cpu(t) in cores over 61 samples, memory y, parent)."""
    t = np.linspace(0, 60, SAMPLES)
    procs = []

    def add(name, kind, cpu, mem, birth=0, parent=None, death=None):
        procs.append(dict(name=name, kind=kind, cpu=np.clip(cpu, 0, None), mem=mem, birth=birth,
                          parent=parent, death=death))

    for i in range(10):
        add(f"kworker/{i}", "kernel", 0.002 + 0.01 * RNG.random() * np.ones_like(t), -0.85 + 0.03 * i)
    add("systemd-1", "system", 0.01 + 0 * t, -0.2)
    add("journald-310", "system", 0.05 + 0.03 * np.sin(t / 7), -0.35)
    add("postgres-41", "system", 0.45 + 0.25 * np.sin(t / 9) ** 2, 0.55)
    add("nginx-128", "system", 0.2 + 0.1 * np.sin(t / 3), -0.1)
    add("browser-512", "session", 0.35 + 0.3 * (np.sin(t / 5) > 0.6), 0.85)
    add("language-server-71", "session", 0.98 + 0 * t, 0.4)          # one full core: lightlike
    add("shell-170", "session", 0.01 + 0 * t, -0.6)
    add("make-77", "session", 0.05 + 0 * t, -0.5)
    add("compiler-90", "session", np.where(t > 18, 2.3, 0.3), 0.3)   # multithreaded: superluminal
    add("redis-207", "container", 0.12 + 0 * t, 0.2)
    add("worker-134", "container", 0.6 + 0.35 * np.sin(t / 4) ** 2, 0.1)
    # make -j forks: children branch off make's worldline.
    for j, (b, d) in enumerate([(22, 38), (26, 44), (30, 52), (34, None), (40, None)]):
        add(f"cc1-{900 + j}", "session", 0.95 + 0 * t, -0.3 + 0.12 * j, birth=b, parent="make-77", death=d)
    for p in procs:
        p["kind_index"] = ["kernel", "system", "session", "container"].index(p["kind"])
    order = sorted(range(len(procs)), key=lambda i: (procs[i]["kind_index"], procs[i]["name"]))
    for rank, i in enumerate(order):
        procs[i]["z"] = -0.9 + 1.8 * rank / (len(procs) - 1)
    return procs


def worldline(p, by_name, now):
    """4D points (x, y, z, w) of a process for the window ending at `now` seconds."""
    t = np.linspace(0, 60, SAMPLES)
    cum = np.concatenate([[0], np.cumsum((p["cpu"][1:] + p["cpu"][:-1]) / 2 * np.diff(t))])
    x = -1 + cum / 30.0
    w = -1 + t / 30.0
    y = np.full_like(t, p["mem"])
    z = np.full_like(t, p["z"])
    alive = (t >= p["birth"]) & (t <= now)
    if p["death"] is not None:
        alive &= t <= p["death"]
    if p["parent"]:
        parent = by_name[p["parent"]]
        px, py, pz, _ = worldline_raw(parent)
        b = int(p["birth"])
        # The child starts on its parent's worldline and curves out to its own seat.
        offset = x[b] - px[b]
        x = x - offset
        blend = np.clip((t - p["birth"]) / 3.0, 0, 1)
        blend = blend * blend * (3 - 2 * blend)
        y = py * (1 - blend) + y * blend
        z = pz * (1 - blend) + z * blend
    return np.stack([x, y, z, w], 1)[alive]


def worldline_raw(p):
    t = np.linspace(0, 60, SAMPLES)
    cum = np.concatenate([[0], np.cumsum((p["cpu"][1:] + p["cpu"][:-1]) / 2 * np.diff(t))])
    return -1 + cum / 30.0, np.full_like(t, p["mem"]), np.full_like(t, p["z"]), -1 + t / 30.0


def rotation(angle_xw, angle_yw=0.0, angle_zw=0.0):
    def plane(i, j, a):
        m = np.eye(4)
        c, s = math.cos(a), math.sin(a)
        m[i, i], m[i, j], m[j, i], m[j, j] = c, -s, s, c
        return m
    return plane(0, 3, angle_xw) @ plane(1, 3, angle_yw) @ plane(2, 3, angle_zw)


def project(points, rot, width, height, scale):
    """Rotate in 4D, Schlegel-divide along w, then the isometric orthographic camera."""
    q = points @ rot.T
    f = 1.0 / (EYE - q[:, 3])
    xyz = q[:, :3] * f[:, None] * 2.6
    x, y, z = xyz[:, 0], xyz[:, 2], xyz[:, 1]
    # Orthographic camera turned so CPU (x) runs mostly across the screen and seats recede.
    sx = width * 0.47 + (x * 0.95 - y * 0.42) * scale
    sy = height * 0.47 + (x * 0.18 + y * 0.30) * scale - z * scale
    return sx, sy, f


def draw_segment(canvas, a, b, colour, strength, width_px=1.0):
    """Additive antialiased line by dense splats."""
    n = int(max(abs(b[0] - a[0]), abs(b[1] - a[1])) * 1.5) + 1
    h, w, _ = canvas.shape
    xs = np.linspace(a[0], b[0], n)
    ys = np.linspace(a[1], b[1], n)
    for dx in (0, 1):
        for dy in (0, 1):
            ix = np.clip(xs.astype(int) + dx, 0, w - 1)
            iy = np.clip(ys.astype(int) + dy, 0, h - 1)
            wx = 1 - abs(xs - (xs.astype(int) + dx))
            wy = 1 - abs(ys - (ys.astype(int) + dy))
            wt = (wx * wy * strength)[:, None] * np.array(colour, np.float32) / 255.0 * 200
            np.add.at(canvas, (iy, ix), wt * width_px)


def render(procs, now, rot, width, height, status_lines, labels=True):
    canvas = S.sky(width, height)
    scale = height * 0.20
    by_name = {p["name"]: p for p in procs}
    # Tesseract wireframe: 16 vertices, 32 edges.
    verts = np.array(list(itertools.product([-1, 1], repeat=4)), float)
    sx, sy, f = project(verts, rot, width, height, scale)
    for i, j in itertools.combinations(range(16), 2):
        if np.sum(verts[i] != verts[j]) == 1:
            along_w = verts[i][3] != verts[j][3]
            now_cell = verts[i][3] == 1 and verts[j][3] == 1
            col = (120, 130, 175) if not now_cell else (190, 200, 235)
            draw_segment(canvas, (sx[i], sy[i]), (sx[j], sy[j]), col, 0.35 if along_w else 0.55)
    # Light cone: the diagonal of an x-w face, a process using exactly one core.
    cone = np.array([[-1 + 2 * k / 40, 1.0, 1.0, -1 + 2 * k / 40] for k in range(41)])
    cx, cy, _ = project(cone, rot, width, height, scale)
    for k in range(0, 40, 2):
        draw_segment(canvas, (cx[k], cy[k]), (cx[k + 1], cy[k + 1]), (255, 235, 160), 0.5)
    cone_label = (cx[14], cy[14] - 14)
    heads = []
    for p in procs:
        pts = worldline(p, by_name, now)
        if len(pts) < 2:
            continue
        col = S.KIND[p["kind"]]
        px, py, pf = project(pts, rot, width, height, scale)
        for k in range(len(pts) - 1):
            depth = 0.35 + 0.65 * (pts[k][3] + 1) / 2
            superluminal = pts[k + 1][0] > 1.0
            c = CHERENKOV if superluminal else col
            draw_segment(canvas, (px[k], py[k]), (px[k + 1], py[k + 1]), c, 0.9 * depth, 1.4)
            if superluminal and k % 2 == 0:
                S.glow(canvas, px[k + 1], py[k + 1], 7, CHERENKOV, 0.35)
        dead = p["death"] is not None and now > p["death"]
        if not dead:
            heads.append((p, px[-1], py[-1], pf[-1], pts[-1]))
        else:
            S.glow(canvas, px[-1], py[-1], 6, (255, 255, 255), 0.25)
        if p["parent"] and len(pts) > 1:
            S.glow(canvas, px[0], py[0], 4, (255, 240, 200), 0.6)  # fork vertex
    for p, hx, hy, hf, pt in sorted(heads, key=lambda h: h[3]):
        r = (3.0 + 6.0 * (p["mem"] + 1) / 2) * hf * 3.2
        cpu_now = p["cpu"][min(int(round(pt[3] * 30 + 30)), SAMPLES - 1)]
        if cpu_now > 0.2:
            S.glow(canvas, hx, hy, r * 2.2, S.KIND[p["kind"]], 0.35)
        if cpu_now > 1.0 and pt[0] > 1.0:
            S.glow(canvas, hx, hy, r * 3.5, CHERENKOV, 0.6)
        S.sphere(canvas, hx, hy, r, S.KIND[p["kind"]])
    image = S.to_image(canvas)
    draw = ImageDraw.Draw(image)
    if labels:
        size = max(11, width // 105)
        notes = {
            "language-server-71": "language-server-71  98%  lightlike",
            "compiler-90": "compiler-90  230%  superluminal: Cherenkov",
            "postgres-41": "postgres-41  58%",
            "kworker/0": "kworker/0  at rest",
        }
        for p, hx, hy, hf, pt in heads:
            if p["name"] in notes:
                S.label(draw, hx + 14, hy - 12, notes[p["name"]], size, anchor="lm")
        make = by_name["make-77"]
        pts = worldline(by_name["cc1-900"], by_name, now)
        if len(pts):
            fx, fy, _ = project(pts[:1], rot, width, height, scale)
            S.label(draw, fx[0] - 14, fy[0] + 26, "make -j: fork vertices", size, anchor="rm")
        # Axis legend at the "now" cell.
        S.label(draw, width * 0.035, height * 0.06, "outer cube = now (w = +1)", size, anchor="lm", fill=S.DIM)
        S.label(draw, width * 0.035, height * 0.06 + size * 1.6, "inner cube = 60 s ago (w = -1)", size,
                anchor="lm", fill=S.DIM)
        S.label(draw, cone_label[0] + 10, cone_label[1], "c: one core", size, anchor="lm", fill=(255, 235, 160))
        S.label(draw, width * 0.035, height * 0.06 + size * 3.2, "slope = CPU, c = 1 core", size, anchor="lm",
                fill=S.DIM)
    S.status(image, status_lines)
    return image


STATUS = ["ISOTOP / TESSERACT / DEMO   192 processes   window 60 s   plane XW   lab frame",
          "x = CPU-seconds (30 per unit, slope = CPU) | y = RSS | z = seat | w = time | blue = superluminal (multithreaded)"]


def still():
    procs = make_processes()
    image = render(procs, 60, rotation(0.12, 0.10, -0.08), 1600, 900, STATUS)
    print(S.save_still(image, "tesseract"))


def anim():
    procs = make_processes()
    frames = []
    total = 120
    for i in range(total):
        angle = 0.12 + 2 * math.pi * i / total / 4  # a quarter turn in XW over the clip
        now = 40 + 20 * i / total
        lines = [STATUS[0].replace("plane XW", f"plane XW {math.degrees(angle):5.1f} deg"), STATUS[1]]
        frames.append(render(procs, now, rotation(angle, 0.10, -0.08), 960, 540, lines, labels=i < 30))
    print(S.save_frames(frames, "tesseract", fps=12))


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else "both"
    if mode in ("still", "both"):
        still()
    if mode in ("anim", "both"):
        anim()
