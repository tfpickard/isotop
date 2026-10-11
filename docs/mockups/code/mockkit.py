"""Small drawing kit shared by the anthill, plumbing and ouija mockups.

Everything renders on supersampled float canvases (SS x the output size) with OpenCV's
antialiased primitives, then downsamples with INTER_AREA, so thin strokes and tiny ants stay
clean. Labels and the status panel are drawn afterwards at output size with isostyle.
"""
import math
import os
import subprocess
import sys

import cv2
import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import isostyle as S  # noqa: E402

# Light in world space (x right-down, y left-down, z up), matching S.sphere's screen light
# (-0.5, -0.6) so spheres and world-shaded surfaces agree.
LIGHT = np.array([-0.24, 0.47, 0.85])
LIGHT = LIGHT / np.linalg.norm(LIGHT)
VIEW = np.array([1.0, 1.0, 1.0]) / math.sqrt(3.0)
SHIFT = 4
ONE = 1 << SHIFT


class Geo:
    """2:1 isometric camera over a world square [0, size]^2, on a supersampled canvas."""

    def __init__(self, width, height, ss=2, size=1.0, frac=0.85, top=0.075, centre=0.5):
        self.W, self.H, self.ss, self.size = width, height, ss, size
        self.s = frac * width * ss / 2 / size          # px per world unit along x - y
        self.ox = width * ss * centre
        self.oy = top * height * ss

    def p(self, x, y, z=0.0):
        return (self.ox + (np.asarray(x) - np.asarray(y)) * self.s,
                self.oy + (np.asarray(x) + np.asarray(y)) * self.s * 0.5 - np.asarray(z) * self.s)

    def dirv(self, dx, dy, dz=0.0):
        return ((dx - dy) * self.s, (dx + dy) * self.s * 0.5 - dz * self.s)

    def unproject_ground(self, u, v):
        a = (u - self.ox) / self.s
        b = (v - self.oy) / (self.s * 0.5)
        return (a + b) / 2, (b - a) / 2

    def texture_matrix(self, n):
        """Affine matrix taking an n x n top-down texture of the world square to the screen."""
        k = self.s * self.size / n
        return np.float32([[k, -k, self.ox], [k / 2, k / 2, self.oy]])


def ipt(x, y):
    return (int(round(float(x) * ONE)), int(round(float(y) * ONE)))


def _c(colour):
    return tuple(float(c) for c in colour)


class Layer:
    """Premultiplied RGBA layer in uint8 so OpenCV antialiases (it only does AA on 8-bit).
    Every primitive draws the colour into rgb and 255 into a, which keeps rgb premultiplied."""

    def __init__(self, width, height):
        self.rgb = np.zeros((height, width, 3), np.uint8)
        self.a = np.zeros((height, width), np.uint8)

    def line(self, p0, p1, colour, width=1.0):
        w = max(1, int(round(width)))
        cv2.line(self.rgb, ipt(*p0), ipt(*p1), _c(colour), w, cv2.LINE_AA, SHIFT)
        cv2.line(self.a, ipt(*p0), ipt(*p1), 255, w, cv2.LINE_AA, SHIFT)

    def polyline(self, pts, colour, width=1.0, closed=False):
        arr = np.round(np.asarray(pts, np.float64) * ONE).astype(np.int32).reshape(-1, 1, 2)
        w = max(1, int(round(width)))
        cv2.polylines(self.rgb, [arr], closed, _c(colour), w, cv2.LINE_AA, SHIFT)
        cv2.polylines(self.a, [arr], closed, 255, w, cv2.LINE_AA, SHIFT)

    def poly(self, pts, colour):
        arr = np.round(np.asarray(pts, np.float64) * ONE).astype(np.int32).reshape(-1, 1, 2)
        cv2.fillPoly(self.rgb, [arr], _c(colour), cv2.LINE_AA, SHIFT)
        cv2.fillPoly(self.a, [arr], 255, cv2.LINE_AA, SHIFT)

    def circle(self, c, r, colour, thickness=-1):
        t = thickness if thickness < 0 else max(1, int(round(thickness)))
        rr = max(1, int(round(r * ONE)))
        cv2.circle(self.rgb, ipt(*c), rr, _c(colour), t, cv2.LINE_AA, SHIFT)
        cv2.circle(self.a, ipt(*c), rr, 255, t, cv2.LINE_AA, SHIFT)

    def ellipse(self, c, axes, angle, colour, thickness=-1, start=0, end=360):
        t = thickness if thickness < 0 else max(1, int(round(thickness)))
        ax = (max(1, int(round(axes[0] * ONE))), max(1, int(round(axes[1] * ONE))))
        cv2.ellipse(self.rgb, ipt(*c), ax, angle, start, end, _c(colour), t, cv2.LINE_AA, SHIFT)
        cv2.ellipse(self.a, ipt(*c), ax, angle, start, end, 255, t, cv2.LINE_AA, SHIFT)

    def onto(self, canvas, opacity=1.0):
        a = self.a.astype(np.float32) * (opacity / 255.0)
        canvas *= (1 - a[..., None])
        canvas += self.rgb.astype(np.float32) * opacity

    def add_onto(self, canvas, gain=1.0):
        canvas += self.rgb.astype(np.float32) * gain


def mask_poly(shape, pts):
    """Antialiased coverage (0..1 float) of a polygon."""
    m = np.zeros(shape[:2], np.uint8)
    arr = np.round(np.asarray(pts, np.float64) * ONE).astype(np.int32).reshape(-1, 1, 2)
    cv2.fillPoly(m, [arr], 255, cv2.LINE_AA, SHIFT)
    return m.astype(np.float32) / 255.0


def blend_poly(img, pts, colour, alpha=1.0):
    """Alpha-blends an antialiased filled polygon onto a float canvas (bbox-local)."""
    pts = np.asarray(pts, np.float64)
    x0, y0 = np.floor(pts.min(0)).astype(int) - 2
    x1, y1 = np.ceil(pts.max(0)).astype(int) + 3
    h, w = img.shape[:2]
    x0, y0, x1, y1 = max(x0, 0), max(y0, 0), min(x1, w), min(y1, h)
    if x0 >= x1 or y0 >= y1:
        return
    m = mask_poly((y1 - y0, x1 - x0), pts - [x0, y0])
    region = img[y0:y1, x0:x1]
    a = (m * alpha)[..., None]
    region[:] = region * (1 - a) + np.array(colour, np.float32) * a


def add_glow(img, x, y, radius, colour, strength=1.0):
    S.glow(img, x, y, radius, colour, strength)


def sphere_over(rgb, alpha, x, y, radius, colour, light=(-0.5, -0.6), spec_amt=0.5, rough=30):
    """Premultiplied sphere impostor onto a layer (rgb premultiplied, alpha)."""
    h, w, _ = rgb.shape
    r = int(math.ceil(radius)) + 1
    x0, x1 = max(int(x) - r, 0), min(int(x) + r + 1, w)
    y0, y1 = max(int(y) - r, 0), min(int(y) + r + 1, h)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    dx, dy = (xx - x) / radius, (yy - y) / radius
    d2 = dx * dx + dy * dy
    dz = np.sqrt(np.clip(1 - d2, 0, 1))
    lx, ly = light
    lz = math.sqrt(max(0.0, 1 - lx * lx - ly * ly))
    diffuse = np.clip(dx * lx + dy * ly + dz * lz, 0, 1)
    spec = np.clip(dx * lx * 0.5 + dy * ly * 0.5 + dz * (lz * 0.5 + 0.5), 0, 1) ** rough
    col = np.array(colour, np.float32)
    shade = col * (0.22 + 0.85 * diffuse[..., None]) + 255 * spec_amt * spec[..., None]
    a = np.clip((1.0 - np.sqrt(d2)) * radius, 0, 1)[..., None]
    rgb[y0:y1, x0:x1] = rgb[y0:y1, x0:x1] * (1 - a) + shade * a
    alpha[y0:y1, x0:x1] = alpha[y0:y1, x0:x1] * (1 - a[..., 0]) + a[..., 0]


def composite(canvas, rgb, alpha):
    canvas *= (1 - alpha[..., None])
    canvas += rgb


def downsample(canvas, width, height):
    img = np.clip(canvas, 0, 255).astype(np.float32)
    small = cv2.resize(img, (width, height), interpolation=cv2.INTER_AREA)
    return Image.fromarray(np.clip(small + 0.5, 0, 255).astype(np.uint8))


def value_noise(n, cells, rng, octaves=((1.0, 1.0),)):
    """Smooth noise on an n x n grid from bicubically upsampled random lattices."""
    out = np.zeros((n, n), np.float32)
    for freq, amp in octaves:
        c = max(2, int(cells * freq))
        lat = rng.standard_normal((c, c)).astype(np.float32)
        out += amp * cv2.resize(lat, (n, n), interpolation=cv2.INTER_CUBIC)
    return out


def contact_sheet(frames, path, cols=2, width=960):
    """Grid of PIL frames, for reviewing an animation."""
    frames = [f.convert("RGB") for f in frames]
    w = width // cols
    h = round(frames[0].height * w / frames[0].width)
    rows = math.ceil(len(frames) / cols)
    sheet = Image.new("RGB", (w * cols, h * rows), (0, 0, 0))
    for i, f in enumerate(frames):
        sheet.paste(f.resize((w, h), Image.LANCZOS), ((i % cols) * w, (i // cols) * h))
    sheet.save(path)
    return path


def reencode_gif(name, fps=12, width=720, colors=128, dither="bayer:bayer_scale=4"):
    """Re-encodes OUT/<name>.gif from OUT/<name>-frames/ with one global palette and per-frame
    change rectangles (ffmpeg), which is far smaller than per-frame adaptive palettes."""
    frame_dir = os.path.join(S.OUT, f"{name}-frames")
    out = os.path.join(S.OUT, f"{name}.gif")
    pal = os.path.join(S.OUT, f".{name}-palette.png")
    scale = f"scale={width}:-1:flags=lanczos"
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i",
                    os.path.join(frame_dir, "%04d.png"), "-vf",
                    f"{scale},palettegen=max_colors={colors}:stats_mode=full", pal], check=True)
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i",
                    os.path.join(frame_dir, "%04d.png"), "-i", pal, "-lavfi",
                    f"{scale}[x];[x][1:v]paletteuse=dither={dither}:diff_mode=rectangle",
                    "-loop", "0", out], check=True)
    os.remove(pal)
    return out, os.path.getsize(out)


def save_animation(frames, name, fps=12, limit=3.5e6, attempts=None):
    """S.save_frames first (it writes the PNG frames); if the GIF is over the limit, re-encode
    with a global palette, stepping down colours and width until it fits."""
    path, size = S.save_frames(frames, name, fps=fps)
    print(f"save_frames: {size / 1e6:.2f} MB")
    if size <= limit:
        return path, size
    attempts = attempts or [(720, 128, "bayer:bayer_scale=4"), (720, 96, "bayer:bayer_scale=3"),
                            (640, 96, "bayer:bayer_scale=3"), (640, 64, "bayer:bayer_scale=2"),
                            (560, 64, "none")]
    for width, colors, dither in attempts:
        path, size = reencode_gif(name, fps, width, colors, dither)
        print(f"reencode {width}px {colors}c {dither}: {size / 1e6:.2f} MB")
        if size <= limit:
            break
    return path, size
