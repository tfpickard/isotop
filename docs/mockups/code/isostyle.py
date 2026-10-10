"""Shared look for isotop view mockups, so every reference image reads as the same app.

Import from a mockup script with:
    import sys; sys.path.insert(0, "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto")
    import isostyle as S

Conventions (match isotop's real screenshots):
- Canvas 1600x900 for stills; animations rendered at 960x540 (or 1280x720) and saved <= 720 wide.
- Background: dark navy-purple sky with a soft magenta glow upper-left (S.sky).
- Bottom status panel: 2 to 3 lines of monospace text, the first starting "ISOTOP / <VIEW> / DEMO".
- Process "kind" colours: S.KIND["kernel"|"system"|"session"|"container"], plus S.ZOMBIE pink.
- Labels: monospace, light grey with a 1px dark shadow (S.label).
- Work in float numpy RGB canvases (0..255) for additive glows, convert with S.to_image.
"""
import math
import os

import numpy as np
from PIL import Image, ImageDraw, ImageFont

OUT = "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/out"

KIND = {
    "kernel": (124, 138, 165),     # slate
    "system": (46, 196, 196),      # teal
    "session": (242, 176, 64),     # amber
    "container": (172, 128, 242),  # violet
}
ZOMBIE = (232, 92, 128)
STOPPED = (240, 140, 60)
TEXT = (210, 214, 226)
DIM = (140, 146, 165)
PANEL = (18, 18, 26)

FONT_MONO = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
FONT_MONO_BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
FONT_SERIF = "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf"
FONT_SERIF_BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSerif-Bold.ttf"


def font(size, path=FONT_MONO):
    if os.path.exists(path):
        return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def sky(width, height, glow=(150, 60, 140), base=(10, 9, 22), centre=(0.18, 0.30)):
    """isotop's sky: dark base with a soft coloured glow. Returns a float canvas (H, W, 3)."""
    yy, xx = np.mgrid[0:height, 0:width].astype(np.float32)
    g = np.exp(-(((xx - width * centre[0]) / (width * 0.38)) ** 2
                 + ((yy - height * centre[1]) / (height * 0.5)) ** 2))
    canvas = np.array(base, np.float32) + g[..., None] * (np.array(glow, np.float32) * 0.6)
    return canvas


def to_image(canvas):
    return Image.fromarray(np.clip(canvas, 0, 255).astype(np.uint8))


def to_canvas(image):
    return np.asarray(image.convert("RGB"), np.float32).copy()


def glow(canvas, x, y, radius, colour, strength=1.0):
    """Additive Gaussian glow at (x, y) in pixels."""
    h, w, _ = canvas.shape
    r = int(radius * 3)
    x0, x1 = max(int(x) - r, 0), min(int(x) + r + 1, w)
    y0, y1 = max(int(y) - r, 0), min(int(y) + r + 1, h)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    g = np.exp(-((xx - x) ** 2 + (yy - y) ** 2) / (2 * radius * radius)) * strength
    canvas[y0:y1, x0:x1] += g[..., None] * np.array(colour, np.float32)


def sphere(canvas, x, y, radius, colour, light=(-0.5, -0.6)):
    """Shaded sphere impostor like isotop's planets and pebbles."""
    h, w, _ = canvas.shape
    r = int(math.ceil(radius)) + 1
    x0, x1 = max(int(x) - r, 0), min(int(x) + r + 1, w)
    y0, y1 = max(int(y) - r, 0), min(int(y) + r + 1, h)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    dx, dy = (xx - x) / radius, (yy - y) / radius
    d2 = dx * dx + dy * dy
    inside = d2 <= 1.0
    dz = np.sqrt(np.clip(1 - d2, 0, 1))
    lx, ly = light
    lz = math.sqrt(max(0.0, 1 - lx * lx - ly * ly))
    diffuse = np.clip(dx * lx + dy * ly + dz * lz, 0, 1)
    spec = np.clip(dx * lx * 0.5 + dy * ly * 0.5 + dz * (lz * 0.5 + 0.5), 0, 1) ** 30
    col = np.array(colour, np.float32)
    shade = col * (0.25 + 0.85 * diffuse[..., None]) + 255 * 0.5 * spec[..., None]
    edge = np.clip((1.0 - np.sqrt(d2)) * radius, 0, 1)[..., None]  # 1px antialias
    region = canvas[y0:y1, x0:x1]
    region[:] = np.where(inside[..., None], region * (1 - edge) + shade * edge, region)


def iso(x, y, z, origin, scale):
    """World to screen, 2:1 isometric: +x runs right-down, +y left-down, +z up."""
    ox, oy = origin
    return ox + (x - y) * scale, oy + (x + y) * scale * 0.5 - z * scale


def label(draw, x, y, text, size=14, fill=TEXT, anchor="mm", path=FONT_MONO):
    f = font(size, path)
    draw.text((x + 1, y + 1), text, font=f, fill=(0, 0, 0), anchor=anchor)
    draw.text((x, y), text, font=f, fill=fill, anchor=anchor)


def status(image, lines, size=None):
    """isotop's bottom status panel: a dark band with monospace lines."""
    w, h = image.size
    size = size or max(11, w // 115)
    line_h = int(size * 1.45)
    band = line_h * len(lines) + 10
    draw = ImageDraw.Draw(image)
    draw.rectangle([0, h - band, w, h], fill=PANEL)
    f = font(size)
    for i, text in enumerate(lines):
        draw.text((8, h - band + 5 + i * line_h), text, font=f, fill=TEXT if i == 0 else DIM)
    return image


def save_still(image, name):
    path = os.path.join(OUT, f"{name}.png")
    image.convert("RGB").save(path)
    return path


def save_frames(frames, name, fps=12, max_width=720, colors=160):
    """Saves the PNG frames under OUT/<name>-frames/ (for video encoding) and a GIF at
    OUT/<name>.gif no wider than max_width. Aim for <= 3.5 MB: fewer frames, smaller width or
    fewer colours if it comes out larger."""
    frame_dir = os.path.join(OUT, f"{name}-frames")
    os.makedirs(frame_dir, exist_ok=True)
    for old in os.listdir(frame_dir):
        os.remove(os.path.join(frame_dir, old))
    small = []
    for i, frame in enumerate(frames):
        frame = frame.convert("RGB")
        frame.save(os.path.join(frame_dir, f"{i:04d}.png"))
        if frame.width > max_width:
            frame = frame.resize((max_width, round(frame.height * max_width / frame.width)), Image.LANCZOS)
        small.append(frame.convert("P", palette=Image.ADAPTIVE, colors=colors))
    path = os.path.join(OUT, f"{name}.gif")
    small[0].save(path, save_all=True, append_images=small[1:], duration=int(1000 / fps), loop=0,
                  optimize=True)
    return path, os.path.getsize(path)
