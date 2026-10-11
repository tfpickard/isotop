"""Prototype of isotop's morph view: Gray-Scott reaction-diffusion where every process owns a patch
of the dish and its CPU picks the local feed and kill rates, so the pattern's morphology reads the
process's state. Renders a labelled isometric still and an animation with a process ramping up,
a birth and an exit.

usage: python3 -I morph.py OUTDIR
"""
import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage

SIZE = 300
DU, DV = 0.2097 / 0.3, 0.105 / 0.3

# Pearson-class anchors along CPU (percent of one core), calm to frantic.
ANCHORS = [
    (0.0, 0.014, 0.054, "drifting spots"),
    (8.0, 0.030, 0.062, "solitons"),
    (30.0, 0.037, 0.060, "fingerprint"),
    (80.0, 0.029, 0.057, "maze"),
    (160.0, 0.039, 0.058, "holes"),
    (300.0, 0.018, 0.047, "turbulence"),
]

KIND = {
    "kernel": (124, 138, 165),
    "system": (46, 196, 196),
    "session": (242, 176, 64),
    "container": (172, 128, 242),
}


def feed_kill(cpu):
    """Interpolates the anchors on a log CPU axis."""
    x = math.log1p(cpu)
    for (c0, f0, k0, n0), (c1, f1, k1, n1) in zip(ANCHORS, ANCHORS[1:]):
        a, b = math.log1p(c0), math.log1p(c1)
        if x <= b:
            t = (x - a) / (b - a)
            return f0 + (f1 - f0) * t, k0 + (k1 - k0) * t, (n0 if t < 0.5 else n1)
    return ANCHORS[-1][1], ANCHORS[-1][2], ANCHORS[-1][3]


def laplacian(a):
    return ndimage.convolve(a, KERNEL, mode="wrap")


KERNEL = np.array([[0.05, 0.2, 0.05], [0.2, -1.0, 0.2], [0.05, 0.2, 0.05]])


class Process:
    def __init__(self, name, kind, x, y, memory_mib, cpu):
        self.name, self.kind = name, kind
        self.x, self.y = x * SIZE, y * SIZE
        self.memory = memory_mib
        self.cpu = cpu
        self.alive = True
        self.exiting = 0.0


def demo_processes():
    rng = np.random.default_rng(7)
    names = [
        ("systemd", "system", 80), ("journald", "system", 60), ("dbus", "system", 12),
        ("postgres", "system", 900), ("nginx", "system", 140), ("kworker", "kernel", 4),
        ("ksoftirqd", "kernel", 2), ("browser", "session", 2400), ("compiler", "session", 700),
        ("language-server", "session", 1100), ("shell", "session", 9), ("redis", "container", 300),
        ("worker", "container", 450), ("tracker", "session", 60),
    ]
    procs = []
    golden = math.pi * (3 - math.sqrt(5))
    for i, (name, kind, mem) in enumerate(names):
        r = 0.36 * math.sqrt((i + 0.5) / len(names))
        a = i * golden
        procs.append(Process(f"{name}-{100 + i * 7}", kind, 0.5 + r * math.cos(a), 0.5 + r * math.sin(a),
                             mem, float(rng.choice([0.5, 3, 12, 25, 60, 110, 190]))))
    by = {p.name.split("-")[0]: p for p in procs}
    by["browser"].cpu = 120
    by["compiler"].cpu = 2
    by["language"].cpu = 45
    by["postgres"].cpu = 8
    by["kworker"].cpu = 0.3
    by["ksoftirqd"].cpu = 0.1
    by["worker"].cpu = 260
    by["redis"].cpu = 15
    by["shell"].cpu = 0.2
    by["dbus"].cpu = 1
    by["tracker"].cpu = 70
    return procs


def owners(procs):
    """Power diagram: each cell belongs to the process minimising |x - p|^2 - weight, with weight
    from memory, so heavier processes claim more of the dish."""
    yy, xx = np.mgrid[0:SIZE, 0:SIZE].astype(np.float32)
    best = np.full((SIZE, SIZE), np.inf, np.float32)
    owner = np.full((SIZE, SIZE), -1, np.int32)
    for i, p in enumerate(procs):
        if not p.alive:
            continue
        dx = np.minimum(abs(xx - p.x), SIZE - abs(xx - p.x))
        dy = np.minimum(abs(yy - p.y), SIZE - abs(yy - p.y))
        weight = 60.0 * math.log1p(p.memory)
        d = dx * dx + dy * dy - weight
        closer = d < best
        best[closer] = d[closer]
        owner[closer] = i
    return owner


class Dish:
    def __init__(self, procs, seed=3):
        self.procs = procs
        self.rng = np.random.default_rng(seed)
        self.u = np.ones((SIZE, SIZE))
        self.v = np.zeros((SIZE, SIZE))
        self.owner = owners(procs)
        for p in procs:
            self.seed_patch(procs.index(p), 10)

    def seed_patch(self, index, count):
        ys, xs = np.nonzero(self.owner == index)
        if len(xs) == 0:
            return
        for j in self.rng.integers(0, len(xs), count):
            x, y = xs[j], ys[j]
            self.u[max(y - 2, 0):y + 2, max(x - 2, 0):x + 2] = 0.5
            self.v[max(y - 2, 0):y + 2, max(x - 2, 0):x + 2] = 0.25

    def parameters(self):
        f = np.zeros((SIZE, SIZE))
        k = np.zeros((SIZE, SIZE))
        for i, p in enumerate(self.procs):
            mask = self.owner == i
            if not mask.any():
                continue
            if p.alive:
                fi, ki, _ = feed_kill(p.cpu)
            else:
                # An exited process stops feeding its patch, so its pattern starves and fades.
                fi, ki = 0.0, 0.06
            f[mask], k[mask] = fi, ki
        return ndimage.gaussian_filter(f, 1.5, mode="wrap"), ndimage.gaussian_filter(k, 1.5, mode="wrap")

    def run(self, steps):
        f, k = self.parameters()
        for _ in range(steps):
            uvv = self.u * self.v * self.v
            self.u += DU * laplacian(self.u) - uvv + f * (1 - self.u)
            self.v += DV * laplacian(self.v) + uvv - (f + k) * self.v
        # A patch whose pattern has died out while its process lives is reseeded, as a dish
        # nucleates again.
        for i, p in enumerate(self.procs):
            mask = self.owner == i
            if p.alive and mask.any() and self.v[mask].max() < 0.05:
                self.seed_patch(i, 6)


def font(size):
    for path in ["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                 "/usr/share/fonts/TTF/DejaVuSansMono.ttf"]:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def shade(dish):
    """Colour each cell by its owner's kind, brightness and relief from the activator V."""
    v = dish.v
    h = np.clip(v / 0.33, 0, 1)
    gy, gx = np.gradient(ndimage.gaussian_filter(h, 0.6, mode="wrap"))
    normal = np.dstack([-gx * 6, -gy * 6, np.ones_like(h)])
    normal /= np.linalg.norm(normal, axis=2, keepdims=True)
    light = np.array([-0.5, -0.6, 0.62])
    light /= np.linalg.norm(light)
    diffuse = np.clip((normal * light).sum(2), 0, 1)
    half = light + np.array([0, 0, 1.0])
    half /= np.linalg.norm(half)
    specular = np.clip((normal * half).sum(2), 0, 1) ** 40
    base = np.zeros((SIZE, SIZE, 3))
    for i, p in enumerate(dish.procs):
        mask = dish.owner == i
        base[mask] = KIND[p.kind]
    agar = np.array([14, 22, 34], float)
    t = h[..., None] ** 0.8
    colour = agar * (1 - t) + base * t * (0.45 + 0.75 * diffuse[..., None])
    colour += 255 * 0.55 * specular[..., None] * t
    # Faint patch boundaries.
    edge = (dish.owner != np.roll(dish.owner, 1, 0)) | (dish.owner != np.roll(dish.owner, 1, 1))
    colour[edge] = colour[edge] * 0.6 + np.array([90, 100, 130]) * 0.4
    return np.clip(colour, 0, 255).astype(np.uint8)


def sky(width, height):
    yy, xx = np.mgrid[0:height, 0:width].astype(np.float32)
    base = np.array([10, 9, 22], float)
    glow = np.exp(-(((xx - width * 0.18) / (width * 0.35)) ** 2 + ((yy - height * 0.3) / (height * 0.45)) ** 2))
    img = base + glow[..., None] * np.array([90, 40, 100], float)
    return Image.fromarray(np.clip(img, 0, 255).astype(np.uint8))


def isometric(top, width, height):
    """Rotates the square dish 45 degrees and halves its height, the 2:1 isometric diamond."""
    im = Image.fromarray(top).resize((SIZE * 3, SIZE * 3), Image.BICUBIC)
    im = im.rotate(45, resample=Image.BICUBIC, expand=True, fillcolor=(0, 0, 0))
    mask = Image.fromarray((np.ones((SIZE * 3, SIZE * 3)) * 255).astype(np.uint8)).rotate(
        45, resample=Image.BICUBIC, expand=True)
    w = int(width * 0.86)
    h = w // 2
    im, mask = im.resize((w, h), Image.LANCZOS), mask.resize((w, h), Image.LANCZOS)
    canvas = sky(width, height)
    shadow = Image.new("RGB", (w, h), (0, 0, 0))
    ox, oy = (width - w) // 2, int(height * 0.1)
    canvas.paste(shadow, (ox + 6, oy + 14), mask.point(lambda a: a * 0.5))
    canvas.paste(im, (ox, oy), mask)
    return canvas, (ox, oy, w, h)


def project(x, y, frame):
    """Dish coordinates to the isometric canvas."""
    ox, oy, w, h = frame
    u, v = x / SIZE - 0.5, y / SIZE - 0.5
    # PIL rotates counterclockwise: the dish's top-left corner becomes the diamond's left vertex.
    return ox + w / 2 + (u + v) * w / 2, oy + h / 2 + (v - u) * h / 2


def render(dish, width, height, labels, status):
    canvas, frame = isometric(shade(dish), width, height)
    draw = ImageDraw.Draw(canvas)
    small = font(max(11, width // 110))
    if labels:
        for i, p in enumerate(dish.procs):
            ys, xs = np.nonzero(dish.owner == i)
            if len(xs) < 40:
                continue
            # Circular means, because the dish wraps around.
            ax = np.angle(np.exp(2j * np.pi * xs / SIZE).mean()) % (2 * np.pi) * SIZE / (2 * np.pi)
            ay = np.angle(np.exp(2j * np.pi * ys / SIZE).mean()) % (2 * np.pi) * SIZE / (2 * np.pi)
            cx, cy = project(ax, ay, frame)
            _, _, name = feed_kill(p.cpu)
            text = f"{p.name} {p.cpu:.0f}% {name}" if p.alive else f"{p.name} exited"
            draw.text((cx + 1, cy + 1), text, font=small, fill=(0, 0, 0), anchor="mm")
            draw.text((cx, cy), text, font=small, fill=(225, 228, 240), anchor="mm")
    strip = font(max(12, width // 100))
    draw.rectangle([0, height - 34, width, height], fill=(18, 18, 26))
    draw.text((10, height - 27), status, font=strip, fill=(200, 205, 215))
    return canvas


def main(outdir):
    os.makedirs(os.path.join(outdir, "morph-frames"), exist_ok=True)
    procs = demo_processes()
    dish = Dish(procs)
    for _ in range(40):
        dish.run(200)
    still = render(dish, 1600, 900, True,
                   "ISOTOP / MORPH   patch = process (area ~ memory) | pattern = CPU: drifting spots, solitons, fingerprint, maze, holes, turbulence")
    still.save(os.path.join(outdir, "morph-still.png"))
    print("still done")

    by = {p.name.split("-")[0]: p for p in procs}
    compiler = by["compiler"]
    frames = []
    total = 150
    newborn = None
    for frame_index in range(total):
        phase = frame_index / total
        # The compiler ramps from idle to three busy cores and back.
        compiler.cpu = 2 + 300 * math.sin(math.pi * min(1.0, phase / 0.8)) ** 2
        if frame_index == 45:
            newborn = Process("ffmpeg-999", "session", 0.78, 0.30, 500, 140)
            procs.append(newborn)
            dish.owner = owners(procs)
            dish.seed_patch(len(procs) - 1, 12)
        if frame_index == 70:
            by["tracker"].alive = False
        dish.run(60)
        image = render(dish, 880, 500, True,
                       f"ISOTOP / MORPH   compiler {compiler.cpu:.0f}% -> {feed_kill(compiler.cpu)[2]}"
                       + ("   ffmpeg born" if 45 <= frame_index < 70 else "")
                       + ("   tracker exited: its patch starves" if frame_index >= 70 else ""))
        frames.append(image.convert("P", palette=Image.ADAPTIVE, colors=160))
        image.save(os.path.join(outdir, "morph-frames", f"{frame_index:04d}.png"))
        if frame_index % 25 == 0:
            print("frame", frame_index)
    frames[0].save(os.path.join(outdir, "morph.gif"), save_all=True, append_images=frames[1:],
                   duration=80, loop=0, optimize=True)
    print("gif done")


if __name__ == "__main__":
    main(sys.argv[1])
