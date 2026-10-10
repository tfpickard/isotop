"""Mockup of isotop's ouija view: the system journal spelled out on a candlelit talking board.

A dark wooden talking board lies on a velvet table, seen in perspective by candlelight. A
heart-shaped planchette with a glass window glides between letters; the letter under the window
glows and is magnified by the lens. Before a line of priority err or worse, the planchette swings
to NO; then it spells the line letter by letter while a terminal ribbon below the board decodes
it. Jitter grows with CPU pressure ("spirits restless"). Candles are decorative.

Everything on the board is painted in board texture space and warped with a real perspective
camera, so lettering foreshortens with the board.

usage: python3 -I ouija.py [still|anim|both]
"""
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

BW, BH = 2400, 1500             # board texture
BOARD_X, BOARD_Y = 1.6, 1.0     # board size in world units
YAW = math.radians(-4)
CAM = np.array([0.05, -2.25, 1.95])
TARGET = np.array([0.0, 0.02, 0.0])
CANDLES = [  # world x, y, height, radius
    (-1.02, 0.08, 0.24, 0.058),
    (1.02, 0.20, 0.19, 0.052),
]
PAINT = np.array([226, 204, 156], np.float32)
FONT_LET = "/usr/share/fonts/truetype/dejavu/DejaVuSerifCondensed-Bold.ttf"
FONT_WORD = "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf"

LINE = "kernel: nvme0: I/O 42 QID 6 timeout"
PREFIX = 8                      # "kernel: " is known before the spelling starts
SPELL = ["N", "V", "M", "E", "0", "I", "O", "4", "2"]
REVEAL = [9, 10, 11, 12, 15, 17, 19, 20, 21]   # characters of LINE shown after each target
T_NO = (0.35, 1.55, 2.45)       # leave, arrive at NO, leave NO
STEP_MOVE, STEP_DWELL = 0.52, 0.34
PRESSURE = 0.18


def rot_z(v, a):
    c, s = math.cos(a), math.sin(a)
    return np.stack([c * v[..., 0] - s * v[..., 1], s * v[..., 0] + c * v[..., 1], v[..., 2]], -1)


class Camera:
    def __init__(self, width, height, ss):
        self.W, self.H, self.ss = width * ss, height * ss, ss
        f = TARGET - CAM
        self.f = f / np.linalg.norm(f)
        r = np.cross(self.f, [0, 0, 1.0])
        self.r = r / np.linalg.norm(r)
        self.u = np.cross(self.r, self.f)
        self.F = 1.12 * self.W
        self.cx, self.cy = self.W / 2, self.H * 0.37

    def project(self, P):
        P = np.asarray(P, float)
        d = P - CAM
        z = d @ self.f
        x = self.cx + self.F * (d @ self.r) / z
        y = self.cy - self.F * (d @ self.u) / z
        return np.stack([x, y], -1), z

    def board_world(self, u, v, z=0.0):
        """Board texture px -> world point (the board is yawed about its centre)."""
        X = (np.asarray(u) / BW - 0.5) * BOARD_X
        Y = (0.5 - np.asarray(v) / BH) * BOARD_Y
        P = np.stack([X, Y, np.full_like(np.asarray(X, float), z)], -1)
        return rot_z(P, YAW)

    def homography(self, w, h, to_world):
        src = np.float32([[0, 0], [w, 0], [w, h], [0, h]])
        dst, _ = self.project(to_world(src[:, 0], src[:, 1]))
        return cv2.getPerspectiveTransform(src, np.float32(dst))


# ---------------------------------------------------------------------------------------------
# Static board texture


def wood(w, h, rng, base=(64, 38, 24)):
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    warp = K.value_noise(max(w, h), 6, rng, ((1, 1.0), (2.5, 0.4)))[:h, :w]
    rings = np.sin((yy + 70 * warp) * 0.09 + 3 * np.sin(xx * 0.0016))
    rings2 = np.sin((yy * 1.7 + 50 * warp) * 0.11)
    fib = rng.standard_normal((h, w)).astype(np.float32)
    fib = cv2.GaussianBlur(fib, (0, 0), sigmaX=40, sigmaY=0.9) * 6
    tone = 0.82 + 0.10 * rings + 0.05 * rings2 + 0.10 * fib + 0.08 * warp
    knots = np.zeros((h, w), np.float32)
    for _ in range(3):
        kx, ky = rng.uniform(0.1, 0.9) * w, rng.uniform(0.1, 0.9) * h
        d = np.hypot((xx - kx) / 3.0, yy - ky)
        knots += np.exp(-d / 40) * 0.25
    tex = np.array(base, np.float32) * (tone - knots)[..., None]
    return tex


def letter_layout():
    """Positions (texture px), rotations and sizes of every glyph on the board."""
    items = []
    cx, cy = BW / 2, 2050
    for chars, R, a0, a1, size in (("ABCDEFGHIJKLM", 1520, 122, 58, 150), ("NOPQRSTUVWXYZ", 1300, 120, 60, 134)):
        for i, ch in enumerate(chars):
            th = math.radians(a0 + (a1 - a0) * i / (len(chars) - 1))
            x, y = cx + R * math.cos(th), cy - R * math.sin(th)
            items.append((ch, x, y, math.degrees(th) - 90, size, FONT_LET))
    for i, ch in enumerate("1234567890"):
        x = 560 + i * (BW - 1120) / 9
        items.append((ch, x, 1050, 0.0, 118, FONT_LET))
    items.append(("YES", 520, 250, 0.0, 120, FONT_WORD))
    items.append(("NO", 1900, 250, 0.0, 120, FONT_WORD))
    items.append(("GOODBYE", BW / 2, 1255, 0.0, 112, FONT_WORD))
    return items


def glyph(ch, size, angle, path):
    f = ImageFont.truetype(path, size)
    l, t, r, b = f.getbbox(ch)
    pad = 20
    img = Image.new("L", (r - l + 2 * pad, b - t + 2 * pad), 0)
    ImageDraw.Draw(img).text((pad - l, pad - t), ch, font=f, fill=255)
    if ch in ("YES", "NO", "GOODBYE"):
        pass
    img = img.rotate(angle, resample=Image.BICUBIC, expand=True)
    return np.asarray(img, np.float32) / 255.0


def sun_moon(mask):
    """Line-art sun (with YES) and moon (with NO) ornaments, plus border and corner flourishes."""
    img = Image.fromarray((mask * 255).astype(np.uint8))
    d = ImageDraw.Draw(img)
    sx, sy, r = 230, 255, 78
    d.ellipse([sx - r, sy - r, sx + r, sy + r], outline=255, width=9)
    d.ellipse([sx - r * 0.18 - 26, sy - 22, sx - r * 0.18 - 10, sy - 6], fill=255)
    d.ellipse([sx + r * 0.18 + 10, sy - 22, sx + r * 0.18 + 26, sy - 6], fill=255)
    d.arc([sx - 34, sy - 10, sx + 34, sy + 40], 20, 160, fill=255, width=7)
    for k in range(16):
        a = k / 16 * 2 * math.pi
        r0, r1 = r + 18, r + (62 if k % 2 == 0 else 38)
        w = 0.09 if k % 2 == 0 else 0.07
        d.polygon([(sx + r0 * math.cos(a - w * 1.6), sy + r0 * math.sin(a - w * 1.6)),
                   (sx + r1 * math.cos(a), sy + r1 * math.sin(a)),
                   (sx + r0 * math.cos(a + w * 1.6), sy + r0 * math.sin(a + w * 1.6))], fill=255)
    mx, my, r = BW - 230, 255, 92
    moon = Image.new("L", img.size, 0)
    md = ImageDraw.Draw(moon)
    md.ellipse([mx - r, my - r, mx + r, my + r], fill=255)
    md.ellipse([mx - r + 52, my - r - 14, mx + r + 40, my + r - 22], fill=0)
    img = Image.fromarray(np.maximum(np.asarray(img), np.asarray(moon)))
    d = ImageDraw.Draw(img)
    for (px, py, s) in ((mx + 70, my - 70, 20), (mx + 100, my + 30, 13), (mx + 40, my + 80, 10)):
        d.polygon([(px, py - s * 2), (px + s * 0.5, py - s * 0.5), (px + s * 2, py), (px + s * 0.5, py + s * 0.5),
                   (px, py + s * 2), (px - s * 0.5, py + s * 0.5), (px - s * 2, py), (px - s * 0.5, py - s * 0.5)], fill=255)
    # Double border with rounded corners.
    d.rounded_rectangle([46, 46, BW - 46, BH - 46], radius=70, outline=255, width=7)
    d.rounded_rectangle([70, 70, BW - 70, BH - 70], radius=54, outline=255, width=3)
    # Flourish under GOODBYE and a small diamond rule at the top centre.
    for sgn in (-1, 1):
        d.line([(BW / 2 + sgn * 330, 1262), (BW / 2 + sgn * 560, 1262)], fill=255, width=4)
        d.polygon([(BW / 2 + sgn * 590, 1262 - 12), (BW / 2 + sgn * 602, 1262), (BW / 2 + sgn * 590, 1262 + 12),
                   (BW / 2 + sgn * 578, 1262)], fill=255)
    d.line([(BW / 2 - 260, 140), (BW / 2 + 260, 140)], fill=255, width=3)
    for k, s in ((0, 16), (-150, 9), (150, 9)):
        x = BW / 2 + k
        d.polygon([(x, 140 - s), (x + s, 140), (x, 140 + s), (x - s, 140)], fill=255)
    return np.asarray(img, np.float32) / 255.0


def board_texture(rng):
    tex = wood(BW, BH, rng)
    paint = np.zeros((BH, BW), np.float32)
    targets, glows = {}, {}
    for ch, x, y, ang, size, path in letter_layout():
        g = glyph(ch, size, ang, path)
        h, w = g.shape
        x0, y0 = int(round(x - w / 2)), int(round(y - h / 2))
        paint[y0:y0 + h, x0:x0 + w] = np.maximum(paint[y0:y0 + h, x0:x0 + w], g)
        targets[ch] = (x, y)
        glows[ch] = (x0, y0, g)
    paint = sun_moon(paint)
    # Worn paint: speckled loss, lighter where hands rest less.
    wear = np.clip(0.78 + 0.35 * K.value_noise(BW, 90, rng)[:BH] + 0.08 * rng.standard_normal((BH, BW)).astype(np.float32), 0, 1)
    pm = paint * wear
    # Slight engraved shadow below-right of the paint.
    sh = np.roll(np.roll(paint, 4, 0), 3, 1)
    tex *= (1 - 0.45 * sh)[..., None]
    tex = tex * (1 - pm[..., None]) + PAINT * (0.8 + 0.2 * wear[..., None]) * pm[..., None]
    # Darken toward the edges like an old varnished board, and round the corners.
    yy, xx = np.mgrid[0:BH, 0:BW].astype(np.float32)
    ex = np.minimum(xx, BW - xx) / BW
    ey = np.minimum(yy, BH - yy) / BH
    tex *= (0.70 + 0.30 * np.clip(np.minimum(ex / 0.06, ey / 0.09), 0, 1))[..., None]
    alpha = np.zeros((BH, BW), np.uint8)
    cv2.rectangle(alpha, (0, 0), (BW - 1, BH - 1), 0)
    a_img = Image.new("L", (BW, BH), 0)
    ImageDraw.Draw(a_img).rounded_rectangle([0, 0, BW - 1, BH - 1], radius=90, fill=255)
    alpha = np.asarray(a_img, np.float32) / 255.0
    targets["YES"] = (520, 250)
    targets["NO"] = (1900, 250)
    return tex, alpha, targets, glows


def heart_poly(n=160):
    """Planchette outline in its own frame (texture px), point toward -v (the far edge)."""
    t = np.linspace(0, 2 * np.pi, n, endpoint=False)
    x = 16 * np.sin(t) ** 3
    y = 13 * np.cos(t) - 5 * np.cos(2 * t) - 2 * np.cos(3 * t) - np.cos(4 * t)
    # Heart with point down in y-up maths coords; on the board we want the point far (-v).
    pts = np.stack([x, y], -1) * 13.0
    pts[:, 1] = pts[:, 1] - 20
    return pts            # y positive = toward far edge after flip below


# ---------------------------------------------------------------------------------------------
# Animation script


def timeline(T):
    """Keyframes (time, target name) for the planchette."""
    keys = [(0.0, "REST"), (T_NO[0], "REST"), (T_NO[1], "NO"), (T_NO[2], "NO")]
    t = T_NO[2]
    for ch in SPELL:
        t += STEP_MOVE
        keys.append((t, ch))
        t += STEP_DWELL
        keys.append((t, ch))
    keys.append((T + 5, keys[-1][1]))
    return keys


def ease(u):
    return u * u * (3 - 2 * u)


class Board:
    def __init__(self, width, height, ss=2):
        self.width, self.height, self.ss = width, height, ss
        self.cam = Camera(width, height, ss)
        rng = np.random.default_rng(13)
        self.tex, self.alpha, self.targets, self.glows = board_texture(rng)
        self.targets["REST"] = (BW / 2 + 60, 1150)
        u, v = np.meshgrid(np.arange(BW, dtype=np.float32) + 0.5, np.arange(BH, dtype=np.float32) + 0.5)
        self.world = self.cam.board_world(u, v).astype(np.float32)
        self.Hb = self.cam.homography(BW, BH, lambda u, v: self.cam.board_world(u, v))
        # Table: velvet cloth around the board, in its own texture.
        self.TW, self.TH = 1400, 1000
        tx0, tx1, ty0, ty1 = -2.9, 2.9, -1.9, 2.1
        self.tbounds = (tx0, tx1, ty0, ty1)
        def table_world(u, v):
            X = tx0 + np.asarray(u) / self.TW * (tx1 - tx0)
            Y = ty1 - np.asarray(v) / self.TH * (ty1 - ty0)
            return np.stack([X, Y, np.full_like(np.asarray(X, float), -0.03)], -1)
        self.Ht = self.cam.homography(self.TW, self.TH, table_world)
        tu, tv = np.meshgrid(np.arange(self.TW, dtype=np.float32) + 0.5, np.arange(self.TH, dtype=np.float32) + 0.5)
        self.tworld = table_world(tu, tv).astype(np.float32)
        nap = K.value_noise(self.TW, 40, rng, ((1, 1.0), (3, 0.4)))[:self.TH, :self.TW]
        folds = np.sin(self.tworld[..., 0] * 3.1 + 2 * K.value_noise(self.TW, 4, rng)[:self.TH, :self.TW]) * 0.10
        self.cloth = np.array([44, 20, 24], np.float32) * (0.85 + 0.12 * nap + folds)[..., None]
        r = np.hypot(self.tworld[..., 0] / 2.6, (self.tworld[..., 1] - 0.1) / 1.7)
        self.tfade = np.clip((1.0 - r) / 0.35, 0, 1).astype(np.float32)
        wss, hss = width * ss, height * ss
        self.sky = S.sky(wss, hss, glow=(150, 60, 140), centre=(0.18, 0.22))
        self.bmask = cv2.warpPerspective(self.alpha, self.Hb, (wss, hss), flags=cv2.INTER_LINEAR)
        self.tmask = cv2.warpPerspective(self.tfade, self.Ht, (wss, hss), flags=cv2.INTER_LINEAR)
        self.heart = heart_poly()
        self.keys = None

    # -----------------------------------------------------------------------------------------

    def candle_flames(self, t):
        out = []
        for i, (x, y, h, r) in enumerate(CANDLES):
            fl = (1 + 0.07 * math.sin(t * 11.3 + i * 2) + 0.05 * math.sin(t * 23.7 + i * 5)
                  + 0.04 * math.sin(t * 5.1 + i))
            out.append((np.array([x, y, h + 0.07]), fl))
        return out

    def light(self, world, t, flames):
        L = np.zeros(world.shape[:2], np.float32)
        for P, fl in flames:
            d = world - P.astype(np.float32)
            d2 = (d ** 2).sum(-1)
            cosi = np.clip(-d[..., 2], 0, None) / np.sqrt(d2)
            L += fl * 0.34 * cosi / (d2 + 0.05)
        return L

    def planchette_state(self, t):
        if self.keys is None:
            self.keys = timeline(12)
        keys = self.keys
        for (t0, a), (t1, b) in zip(keys[:-1], keys[1:]):
            if t0 <= t < t1:
                u = 0 if t1 == t0 else (t - t0) / (t1 - t0)
                pa, pb = np.array(self.targets[a]), np.array(self.targets[b])
                e = ease(u)
                pos = pa + (pb - pa) * e
                # Curving glide: bow the path sideways a little, as a hand-guided planchette does.
                d = pb - pa
                if np.linalg.norm(d) > 1:
                    perp = np.array([-d[1], d[0]]) / np.linalg.norm(d)
                    pos = pos + perp * math.sin(math.pi * e) * 0.12 * np.linalg.norm(d)
                moving = a != b
                cur = b if (not moving or u > 0.85) else None
                vel = (pb - pa) * (6 * u * (1 - u)) / max(t1 - t0, 1e-3)
                break
        else:
            pos, cur, vel, moving = np.array(self.targets[keys[-1][1]]), keys[-1][1], np.zeros(2), False
        # Restless spirits: jitter scaled by CPU pressure.
        j = PRESSURE / 0.18
        pos = pos + j * np.array([7 * math.sin(t * 13.1) + 4 * math.sin(t * 29.3 + 1),
                                  6 * math.sin(t * 11.7 + 2) + 4 * math.sin(t * 31.9)])
        ang = float(np.clip(vel[0] * 0.006, -10, 10)) + 2.0 * math.sin(t * 1.7)
        return pos, cur, ang, moving

    def trail(self, t, n=40, span=1.6):
        pts = []
        for k in range(n):
            tt = t - span * k / n
            if tt < 0:
                break
            p, _, _, _ = self.planchette_state(tt)
            pts.append(p)
        return np.array(pts)

    def revealed(self, t):
        """How many characters of LINE are decoded at time t."""
        n = PREFIX
        tt = T_NO[2]
        for ch, rv in zip(SPELL, REVEAL):
            tt += STEP_MOVE
            if t >= tt - 0.03:
                n = rv
            tt += STEP_DWELL
        return n

    # -----------------------------------------------------------------------------------------

    def board_frame(self, t, flames):
        tex = self.tex
        L = self.light(self.world, t, flames)
        warm = np.array([1.0, 0.78, 0.52], np.float32)
        lit = tex * (0.11 + 2.6 * L[..., None] ** 1.15 * warm)
        # Varnish: flame reflections in the glossy board, smeared toward the viewer.
        for P, fl in flames:
            M = P * np.array([1, 1, -1])
            k = CAM[2] / (CAM[2] + P[2])
            hit = CAM + k * (M - CAM)
            # world -> texture
            q = rot_z(hit[None], -YAW)[0]
            u = (q[0] / BOARD_X + 0.5) * BW
            v = (0.5 - q[1] / BOARD_Y) * BH
            if -200 < u < BW + 200:
                yy, xx = np.ogrid[0:BH, 0:BW]
                g = np.exp(-(((xx - u) / 70.0) ** 2 + ((yy - v) / 260.0) ** 2))
                lit += (g * fl * 70)[..., None].astype(np.float32) * np.array([1.0, 0.75, 0.45], np.float32)
        pos, cur, ang, moving = self.planchette_state(t)
        # Ectoplasm trail of where the planchette has been.
        tr = self.trail(t)
        if len(tr) > 1:
            m = np.zeros((BH // 4, BW // 4), np.float32)
            for i in range(len(tr) - 1):
                a = (1 - i / len(tr)) ** 1.5
                cv2.line(m, tuple(np.int32(tr[i] / 4)), tuple(np.int32(tr[i + 1] / 4)), a, 6, cv2.LINE_AA)
            m = cv2.GaussianBlur(m, (0, 0), 5)
            m = cv2.resize(m, (BW, BH), interpolation=cv2.INTER_LINEAR)
            lit += m[..., None] * np.array([60, 150, 140], np.float32) * 0.9
        # The letter under the window glows.
        if cur is not None:
            key = cur if cur in self.glows else None
            if cur in ("NO", "YES"):
                key = cur
            if key and key in self.glows:
                x0, y0, gmask = self.glows[key]
                h, w = gmask.shape
                pad = 90
                big = np.zeros((h + 2 * pad, w + 2 * pad), np.float32)
                big[pad:pad + h, pad:pad + w] = gmask
                halo = cv2.GaussianBlur(big, (0, 0), 26)
                core = cv2.GaussianBlur(big, (0, 0), 3)
                pulse = 0.85 + 0.15 * math.sin(t * 9)
                ys, xs = y0 - pad, x0 - pad
                y_a, x_a = max(ys, 0), max(xs, 0)
                y_b, x_b = min(ys + big.shape[0], BH), min(xs + big.shape[1], BW)
                sub = lit[y_a:y_b, x_a:x_b]
                hh = halo[y_a - ys:y_b - ys, x_a - xs:x_b - xs]
                cc = core[y_a - ys:y_b - ys, x_a - xs:x_b - xs]
                sub += (hh * 2.6 * pulse)[..., None] * np.array([255, 190, 90], np.float32)
                sub += (cc * 0.9 * pulse)[..., None] * np.array([255, 245, 210], np.float32)
        self.draw_planchette(lit, pos, ang, L)
        return lit, pos, cur

    def draw_planchette(self, lit, pos, ang, L):
        a = math.radians(ang)
        R = np.array([[math.cos(a), -math.sin(a)], [math.sin(a), math.cos(a)]])
        base = self.heart.copy()
        win_c = np.array([0.0, -40.0])    # window centre in the planchette frame
        pts = (base - win_c) @ R.T + pos
        pad = 120
        x0, y0 = np.floor(pts.min(0)).astype(int) - pad
        x1, y1 = np.ceil(pts.max(0)).astype(int) + pad
        x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, BW), min(y1, BH)
        w, h = x1 - x0, y1 - y0
        local = pts - [x0, y0]
        sub = lit[y0:y1, x0:x1]
        def mask(p, blur=0.0):
            m = np.zeros((h, w), np.uint8)
            cv2.fillPoly(m, [np.int32(np.round(p * 16)).reshape(-1, 1, 2)], 255, cv2.LINE_AA, 4)
            m = m.astype(np.float32) / 255
            return cv2.GaussianBlur(m, (0, 0), blur) if blur else m
        # Soft shadow away from the candles (toward the viewer) and a contact shadow.
        sh = mask(local + [10, 46], 22)
        sub *= (1 - 0.62 * sh)[..., None]
        # Side face (thickness), visible toward the viewer.
        side = mask(local + [0, 26])
        top = mask(local)
        sidecol = np.array([120, 92, 70], np.float32) * (0.25 + 1.2 * L[y0:y1, x0:x1, None])
        sub[:] = sub * (1 - side[..., None]) + sidecol * side[..., None]
        # Top face: bone-white lacquer, bevelled, lit by the candles.
        dist = cv2.distanceTransform((top > 0.5).astype(np.uint8), cv2.DIST_L2, 5)
        bevel = np.clip(dist / 26, 0, 1)
        gy, gx = np.gradient(cv2.GaussianBlur(bevel, (0, 0), 3))
        nrm = np.dstack([-gx * 18, -gy * 18, np.ones_like(gx)])
        nrm /= np.linalg.norm(nrm, axis=2, keepdims=True)
        ldir = np.array([-0.15, -0.6, 0.78])
        ldir /= np.linalg.norm(ldir)
        diff = np.clip(nrm @ ldir, 0, 1)
        Ls = L[y0:y1, x0:x1, None]
        topcol = np.array([214, 196, 166], np.float32) * (0.18 + 1.9 * Ls) * (0.55 + 0.6 * diff[..., None])
        # Painted gold pinstripe inset from the edge.
        stripe = np.clip(1 - np.abs(dist - 15) / 2.2, 0, 1)
        topcol = topcol * (1 - stripe[..., None]) + np.array([200, 150, 70], np.float32) * (0.4 + 1.6 * Ls) * stripe[..., None]
        sub[:] = sub * (1 - top[..., None]) + topcol * top[..., None]
        # Glass window: magnifies the (already glowing) board underneath.
        c = pos - [x0, y0]
        wr = 66
        yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
        dx, dy = xx - c[0], yy - c[1]
        rr = np.sqrt(dx * dx + dy * dy)
        inside = rr < wr
        mag = 1.55
        mapx = (c[0] + dx / mag + x0).astype(np.float32)
        mapy = (c[1] + dy / mag + y0).astype(np.float32)
        lens = cv2.remap(self._under, mapx, mapy, cv2.INTER_LINEAR)
        edge = np.clip(wr - rr, 0, 1)
        lens = lens * (0.92 + 0.12 * (1 - (rr / wr) ** 2))[..., None]
        sub[:] = sub * (1 - edge[..., None]) + lens * edge[..., None]
        # Brass rim and glints.
        ring = np.clip(1 - np.abs(rr - wr - 3) / 5.0, 0, 1)
        sub[:] = sub * (1 - ring[..., None]) + np.array([190, 150, 80], np.float32) * (0.4 + 1.8 * Ls) * ring[..., None]
        gl = np.exp(-(((xx - c[0] + 26) / 16) ** 2 + ((yy - c[1] + 28) / 9) ** 2))
        sub += gl[..., None] * np.array([255, 240, 220], np.float32) * 0.75
        gl2 = np.exp(-(((xx - c[0] - 30) / 7) ** 2 + ((yy - c[1] - 34) / 5) ** 2))
        sub += gl2[..., None] * np.array([255, 230, 200], np.float32) * 0.35
        # Felt feet are hidden underneath; a pointer notch at the tip.
        tipw = (np.array([0, -1.0]) @ R.T)
        tip = pts[np.argmin(((pts - pos) @ tipw) * -1)]

    # -----------------------------------------------------------------------------------------

    def candle(self, canvas, i, t, flames):
        cam = self.cam
        x, y, h, r = CANDLES[i]
        (bx, by), z = cam.project([x, y, 0.0])
        (tx, ty), _ = cam.project([x, y, h])
        pr = cam.F * r / z
        hss, wss = canvas.shape[:2]
        x0, x1 = int(bx - pr * 3), int(bx + pr * 3)
        y0, y1 = int(ty - pr * 6), int(by + pr * 1.2)
        x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, wss), min(y1, hss)
        sub = canvas[y0:y1, x0:x1]
        yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
        u = (xx - bx) / pr
        ell = pr * 0.32
        inside = (np.abs(u) < 1) & (yy > ty) & (yy < by + ell * np.sqrt(np.clip(1 - u * u, 0, 1)))
        cov = np.clip((1 - np.abs(u)) * pr, 0, 1) * inside
        # Wax: subsurface glow brightest near the flame, side shading.
        hfrac = np.clip((by - yy) / max(by - ty, 1), 0, 1)
        shade = 0.45 + 0.55 * np.sqrt(np.clip(1 - u * u, 0, 1))
        wax = np.array([236, 214, 178], np.float32)
        glowf = 0.25 + 0.95 * hfrac ** 2.5
        col = wax * (shade * glowf)[..., None]
        # Drips.
        rng = np.random.default_rng(i + 3)
        for k in range(5):
            du = rng.uniform(-0.8, 0.8)
            ln = rng.uniform(0.1, 0.45) * (by - ty)
            drip = (np.abs(u - du) < 0.10) & (yy - ty < ln)
            col[drip] *= 1.12
        sub[:] = sub * (1 - cov[..., None]) + col * cov[..., None]
        # Top: melted pool.
        lay = K.Layer(x1 - x0, y1 - y0)
        lay.ellipse((bx - x0, ty - y0), (pr, ell), 0, (250, 226, 180))
        lay.ellipse((bx - x0, ty - y0 + 1), (pr * 0.72, ell * 0.66), 0, (255, 238, 196))
        lay.onto(sub)
        # Brass holder.
        lay = K.Layer(x1 - x0, y1 - y0)
        lay.ellipse((bx - x0, by - y0 + ell * 0.2), (pr * 1.9, ell * 1.9), 0, (90, 66, 30))
        lay.ellipse((bx - x0, by - y0), (pr * 1.7, ell * 1.6), 0, (170, 128, 58))
        lay.onto(sub)
        sub[:] = sub * (1 - cov[..., None]) + col * cov[..., None]
        # Wick and flame.
        P, fl = flames[i]
        (fx, fy), _ = cam.project(P)
        sway = 0.12 * math.sin(t * 3.3 + i) + 0.08 * math.sin(t * 7.9 + 2 * i)
        fh = pr * 2.6 * fl
        fw = pr * 0.62
        cv2.line(canvas, (int(bx), int(ty)), (int(bx), int(ty - pr * 0.35)), (30, 20, 15), max(1, int(pr * 0.12)), cv2.LINE_AA)
        fy0 = ty - pr * 0.3
        yy2, xx2 = np.mgrid[int(fy0 - fh * 1.3):int(fy0 + fw), int(bx - fw * 2):int(bx + fw * 2)].astype(np.float32)
        vy = (fy0 - yy2) / fh                          # 0 at base, 1 at tip
        cxl = bx + sway * fw * np.clip(vy, 0, None) ** 1.5 * 2
        prof = fw * np.where(vy < 0.25, np.sqrt(np.clip(vy / 0.25, 0, 1)) * 0.95 + 0.05,
                             np.clip(1 - (vy - 0.25) / 0.75, 0, 1) ** 0.8)
        dxl = np.abs(xx2 - cxl)
        inside = np.clip((prof - dxl) + 0.5, 0, 1) * (vy > -0.08) * np.clip((prof - 0.5) * 2, 0, 1)
        core = np.clip(1 - dxl / np.maximum(prof * 0.5, 1e-3), 0, 1) * np.clip(1 - vy * 1.2, 0, 1)
        fcol = (np.array([255, 150, 40], np.float32) * (1 - core[..., None])
                + np.array([255, 250, 225], np.float32) * core[..., None])
        blue = np.clip(1 - vy / 0.2, 0, 1) * np.clip(dxl / np.maximum(prof, 1e-3), 0, 1)
        fcol = fcol * (1 - 0.6 * blue[..., None]) + np.array([80, 110, 255], np.float32) * 0.6 * blue[..., None]
        ys, xs = int(fy0 - fh * 1.3), int(bx - fw * 2)
        if ys >= 0 and xs >= 0 and ys + yy2.shape[0] <= hss and xs + yy2.shape[1] <= wss:
            region = canvas[ys:ys + yy2.shape[0], xs:xs + yy2.shape[1]]
            region[:] = region * (1 - inside[..., None] * 0.95) + fcol * inside[..., None] * 0.95
        S.glow(canvas, bx, fy0 - fh * 0.45, fh * 0.55, (255, 170, 70), 0.55 * fl)
        S.glow(canvas, bx, fy0 - pr * 1.2, pr * 5.7, (255, 130, 50), 0.22)
        S.glow(canvas, bx, fy0 - pr * 1.2, pr * 11.7, (255, 120, 50), 0.06)

    def frame(self, t, status_lines, ribbon=True):
        ss = self.ss
        wss, hss = self.width * ss, self.height * ss
        flames = self.candle_flames(t)
        # Lighting uses steady candles (the flames themselves flicker), so static pixels stay
        # identical between frames and the GIF compresses.
        steady = [(P, 1.0) for P, _ in flames]
        canvas = self.sky.copy()
        # Table cloth.
        Lt = self.light(self.tworld, t, steady)
        cloth = self.cloth * (0.22 + 1.9 * Lt[..., None] * np.array([1.0, 0.8, 0.6], np.float32))
        tw = cv2.warpPerspective(cloth, self.Ht, (wss, hss), flags=cv2.INTER_LINEAR)
        canvas = canvas * (1 - self.tmask[..., None]) + tw * self.tmask[..., None]
        # Board shadow and thickness.
        lay = K.Layer(wss, hss)
        cam = self.cam
        th = 0.035
        c3 = lambda u, v, z: cam.project(cam.board_world(np.float64(u), np.float64(v), z))[0]
        sh = np.zeros((hss, wss), np.float32)
        quad = np.array([c3(u, v, -0.03) for u, v in ((0, 0), (BW, 0), (BW, BH), (0, BH))]) + [10 * ss, 18 * ss]
        cv2.fillPoly(sh, [np.int32(quad).reshape(-1, 1, 2)], 1.0, cv2.LINE_AA)
        sh = cv2.GaussianBlur(sh, (0, 0), 16 * ss)
        canvas *= (1 - 0.7 * sh)[..., None]
        front = [c3(0, BH, 0), c3(BW, BH, 0), c3(BW, BH, -th), c3(0, BH, -th)]
        right = [c3(BW, 0, 0), c3(BW, BH, 0), c3(BW, BH, -th), c3(BW, 0, -th)]
        lay.poly(front, (52, 30, 20))
        lay.poly(right, (34, 20, 14))
        lay.line(front[0], front[1], (120, 84, 54), 1.5 * ss)
        lay.onto(canvas)
        # The board itself.
        self._under = None
        lit_pre = None
        # The lens needs the lit board (with glow) before the planchette is drawn.
        tex_l, pos, cur = self._board_with_lens(t, steady)
        bw = cv2.warpPerspective(tex_l, self.Hb, (wss, hss), flags=cv2.INTER_LINEAR)
        canvas = canvas * (1 - self.bmask[..., None]) + bw * self.bmask[..., None]
        for i in range(len(CANDLES)):
            self.candle(canvas, i, t, flames)
        # Warm vignette.
        yy, xx = np.ogrid[0:hss, 0:wss]
        r = np.sqrt(((xx - wss * 0.5) / (wss * 0.62)) ** 2 + ((yy - hss * 0.42) / (hss * 0.62)) ** 2)
        canvas *= np.clip(1.12 - 0.55 * r ** 2, 0.35, 1.05)[..., None]
        img = K.downsample(canvas, self.width, self.height)
        if ribbon:
            self.draw_ribbon(img, t, cur)
        S.status(img, list(status_lines), size=self.status_size)
        return img

    status_size = None

    def _board_with_lens(self, t, flames):
        # First pass: everything except the planchette, for the lens to sample.
        draw_p = self.draw_planchette
        self.draw_planchette = lambda *a, **k: None
        under, pos, cur = self.board_frame(t, flames)
        self.draw_planchette = draw_p
        self._under = under
        lit = under.copy()
        L = self.light(self.world, t, flames)
        _, _, ang, _ = self.planchette_state(t)
        self.draw_planchette(lit, pos, ang, L)
        return lit, pos, cur

    def draw_ribbon(self, img, t, cur):
        d = ImageDraw.Draw(img)
        sc = self.width / 1600
        compact = self.width < 1200
        fs = max(15, round(22 * sc))
        fsm = max(11, round(14 * sc))
        f = S.font(fs)
        fm = S.font(fsm)
        cw = f.getlength("M")
        n = len(LINE) + 10
        wbox = cw * n + 40 * sc
        x0 = (self.width - wbox) / 2
        y0 = self.height * (0.765 if self.width >= 1200 else 0.745)
        hbox = fs * 1.9
        d.rectangle([x0, y0, x0 + wbox, y0 + hbox], fill=(14, 12, 20), outline=(86, 70, 60))
        d.line([(x0 + 3, y0 + hbox + 2), (x0 + wbox + 3, y0 + hbox + 2)], fill=(0, 0, 0), width=2)
        k = self.revealed(t)
        ts = "01:22:07 "
        tx = x0 + 20 * sc
        ty = y0 + hbox / 2
        d.text((tx, ty), ts, font=f, fill=(110, 110, 130), anchor="lm")
        tx += cw * len(ts)
        rng = np.random.default_rng(int(t * 12))
        pool = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#%&*+=?"
        for i, ch in enumerate(LINE):
            if i < PREFIX:
                col = (200, 184, 160)
                c = ch
            elif i < k:
                col = (255, 214, 140)
                c = ch
            else:
                col = (92, 74, 120)
                c = ch if ch == " " else pool[rng.integers(len(pool))]
            d.text((tx + i * cw, ty), c, font=f, fill=col, anchor="lm")
        if k < len(LINE) and (int(t * 3) % 2 == 0 or True):
            cx = tx + k * cw
            d.rectangle([cx, ty - fs * 0.55, cx + cw * 0.9, ty + fs * 0.55], outline=(255, 214, 140))
        # Caption above the ribbon.
        on_no = cur == "NO" or (T_NO[1] - 0.1 <= t)
        if compact:
            cap = "NO: next line is priority 3 (err)" if on_no else "journal: listening"
        else:
            cap = "journal: next line priority 3 (err), the planchette said NO" if on_no else "journal: listening"
        d.text((x0 + 4, y0 - 6 * sc), cap, font=fm, fill=(170, 150, 140), anchor="ld")
        d.text((x0 + wbox - 4, y0 - 6 * sc), "1 of 3 queued", font=fm, fill=(130, 120, 130), anchor="rd")


def status_lines(t, wide):
    first = "ISOTOP / OUIJA / DEMO   journal 3 queued   spirits restless: cpu pressure 18%"
    if wide:
        return [first,
                "planchette = next journal line, spelled letter by letter | NO = priority err or worse, YES = notice | "
                "ribbon = decoded text | jitter = cpu pressure",
                f"Tab next view | v views | Space pause | f follow unit | p priority filter | q quit   t={88 + t:.1f}s"]
    return [first,
            "planchette spells the next journal line | NO = err or worse, YES = notice",
            f"ribbon = decoded text | jitter = cpu pressure | t={88 + t:.1f}s"]


def main(which):
    if which in ("still", "both"):
        b = Board(1600, 900, ss=2)
        b.status_size = 12
        # On "4", most of "nvme0: I/O" decoded.
        t = T_NO[2] + 8 * (STEP_MOVE + STEP_DWELL) - 0.12
        img = b.frame(t, status_lines(t, True))
        print("still", S.save_still(img, "ouija"))
    if which in ("anim", "both"):
        b = Board(960, 540, ss=2)
        fps, seconds = 12, 11
        frames = []
        for i in range(fps * seconds):
            t = i / fps
            frames.append(b.frame(t, status_lines(t, False)))
            if i % 20 == 0:
                print("frame", i, flush=True)
        path, size = K.save_animation(frames, "ouija", fps=fps)
        K.contact_sheet([frames[i] for i in (10, 24, 60, 110)], os.path.join(S.OUT, "ouija-sheet.png"))
        print("gif", path, f"{size / 1e6:.2f} MB")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "both")
