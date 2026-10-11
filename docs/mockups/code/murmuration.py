"""Prototype of isotop's murmuration view: every thread is a starling in a 3D flock with
topological neighbours (the seven nearest, as real starlings use), cohering to its own process.
A falcon (memory pressure or an OOM kill) dives through and an agitation wave crosses the flock.

usage: python3 -I murmuration.py OUTDIR
"""
import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy import ndimage
from scipy.spatial import cKDTree

RNG = np.random.default_rng(11)
NEIGHBOURS = 7

PROCESSES = [
    # name, kind, threads, busy share of threads
    ("browser-512", "session", 1100, 0.15),
    ("compiler-90", "session", 400, 0.9),
    ("language-server-71", "session", 260, 0.3),
    ("gnome-shell-88", "session", 180, 0.2),
    ("pipewire-95", "session", 60, 0.4),
    ("postgres-41", "system", 220, 0.2),
    ("systemd-1", "system", 40, 0.02),
    ("journald-310", "system", 20, 0.05),
    ("worker-134", "container", 480, 0.6),
    ("redis-207", "container", 90, 0.1),
    ("kworkers", "kernel", 900, 0.05),
]
KIND = {
    "kernel": (124, 138, 165),
    "system": (46, 196, 196),
    "session": (242, 176, 64),
    "container": (172, 128, 242),
}


def font(size):
    for path in ["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"]:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


class Flock:
    def __init__(self):
        owner, kind, busy = [], [], []
        for i, (_, k, threads, share) in enumerate(PROCESSES):
            owner += [i] * threads
            kind += [k] * threads
            busy += list(RNG.random(threads) < share)
        self.owner = np.array(owner)
        self.busy = np.array(busy, float)
        n = len(owner)
        # Each process starts as its own clump inside one loose ball.
        centres = RNG.normal(0, 18, (len(PROCESSES), 3))
        self.position = centres[self.owner] + RNG.normal(0, 14, (n, 3))
        self.position[:, 2] += 60
        heading = RNG.normal(0, 1, 3)
        heading[2] = 0
        self.velocity = heading / np.linalg.norm(heading) + RNG.normal(0, 0.3, (n, 3))
        self.agitation = np.zeros(n)
        # Busy threads fly a little faster, so they surge towards the front of the flock.
        self.speed = 1.0 + 0.3 * self.busy
        self.falcon = None

    def step(self):
        p, v = self.position, self.velocity
        tree = cKDTree(p)
        distance, index = tree.query(p, 3 * NEIGHBOURS + 1)
        # Alignment and separation use the seven nearest, as starlings do; cohesion reaches a
        # little further so groups of eight cannot close off into islands.
        far_index = index[:, 1:]
        distance, index = distance[:, 1:NEIGHBOURS + 1], index[:, 1:NEIGHBOURS + 1]
        # Threads of the same process count double among a bird's seven neighbours, so each
        # process holds together inside the flock without balling up on its own.
        same = (self.owner[index] == self.owner[:, None]).astype(float) + 1.0
        total = same.sum(1, keepdims=True)
        neighbour_v = (v[index] * same[..., None]).sum(1) / total
        neighbour_p = p[far_index].mean(1)
        away = p[:, None, :] - p[index]
        close = (distance < 1.5)[..., None]
        separation = (away / np.maximum(distance[..., None], 0.3) ** 2 * close).sum(1)

        centroid = np.zeros((len(PROCESSES), 3))
        np.add.at(centroid, self.owner, p)
        centroid /= np.bincount(self.owner, minlength=len(PROCESSES))[:, None]
        to_process = centroid[self.owner] - p

        # The roost: a horizontal pull back over it beyond a radius, and an altitude band.
        horizontal = p.copy()
        horizontal[:, 2] = 0
        radius = np.linalg.norm(horizontal, axis=1, keepdims=True)
        roost = -horizontal / np.maximum(radius, 1) * np.clip(radius - 80, 0, None) * 0.004
        roost[:, 2] = np.clip(35 - p[:, 2], 0, None) * 0.01 - np.clip(p[:, 2] - 110, 0, None) * 0.01

        steer = (0.30 * (neighbour_v - v) + 0.02 * (neighbour_p - p) + 0.4 * separation
                 + 0.0006 * (p.mean(0) - p) + roost + RNG.normal(0, 0.02, p.shape))
        # Starlings mostly fly level and turn by banking, which flattens the flock into sheets;
        # a sheet seen edge-on is the dark band of a murmuration.
        steer[:, 2] -= 0.10 * v[:, 2]

        if self.falcon is not None:
            fp, fv = self.falcon
            offset = p - fp
            d = np.linalg.norm(offset, axis=1, keepdims=True)
            near = d[:, 0] < 14
            self.agitation[near] = 1.0
            steer += np.where(d < 14, offset / np.maximum(d, 0.5) * 0.9, 0)
            self.falcon = (fp + fv, fv)
        # Agitation passes from neighbour to neighbour and fades: the wave seen in real
        # murmurations as a dark band rippling across the flock.
        spread = self.agitation[index].max(1) * 0.93
        self.agitation = np.maximum(self.agitation * 0.86, spread * (spread > 0.35))
        steer += self.agitation[:, None] * RNG.normal(0, 0.12, p.shape)

        v = v + steer
        v *= (self.speed / np.maximum(np.linalg.norm(v, axis=1), 1e-6))[:, None]
        self.velocity = v
        self.position = p + v

    def release_falcon(self):
        centre = self.position.mean(0)
        start = centre + np.array([-140.0, 40.0, 70.0])
        direction = centre - start
        self.falcon = (start, direction / np.linalg.norm(direction) * 2.6)


def dusk(width, height):
    t = np.linspace(0, 1, height)[:, None, None]
    top = np.array([28, 34, 70], float)
    middle = np.array([120, 92, 138], float)
    horizon = np.array([238, 150, 104], float)
    colour = np.where(t < 0.6, top + (middle - top) * (t / 0.6), middle + (horizon - middle) * ((t - 0.6) / 0.4))
    return np.repeat(colour, width, axis=1)


def render(flock, width, height, azimuth, highlight, status):
    elevation = math.radians(18)
    ca, sa = math.cos(azimuth), math.sin(azimuth)
    p = flock.position - flock.position.mean(0) * np.array([1, 1, 0])
    x = p[:, 0] * ca - p[:, 1] * sa
    depth = p[:, 0] * sa + p[:, 1] * ca
    y = -p[:, 2] * math.cos(elevation) + depth * math.sin(elevation)
    scale = width / 190
    sx = width / 2 + x * scale
    sy = height * 0.98 + y * scale
    density = np.zeros((height, width))
    lit = np.zeros((height, width))
    inside = (sx >= 0) & (sx < width - 1) & (sy >= 0) & (sy < height - 1)
    weight = 1.0 + 1.6 * flock.agitation
    ix, iy = sx[inside].astype(int), sy[inside].astype(int)
    for dy in (0, 1):
        for dx in (0, 1):
            np.add.at(density, (iy + dy, ix + dx), weight[inside])
    if highlight is not None:
        mine = inside & (flock.owner == highlight)
        np.add.at(lit, (sy[mine].astype(int), sx[mine].astype(int)), 1.0)
    density = ndimage.gaussian_filter(density, 0.7)
    lit = ndimage.gaussian_filter(lit, 1.0)
    alpha = 1 - np.exp(-density * 1.1)
    sky = dusk(width, height)
    bird = np.array([12, 10, 16], float)
    image = sky * (1 - alpha[..., None]) + bird * alpha[..., None]
    if highlight is not None:
        glow = 1 - np.exp(-lit * 2.2)
        image = image * (1 - glow[..., None]) + np.array(KIND[PROCESSES[highlight][1]], float) * glow[..., None]
    # Decorative treeline at the roost.
    xs = np.arange(width)
    ridge = height - 40 - 14 * np.abs(np.sin(xs / 37.0)) - 9 * np.abs(np.sin(xs / 11.0 + 1))
    mask = np.arange(height)[:, None] > ridge[None, :]
    image[mask] = np.array([16, 14, 22])
    canvas = Image.fromarray(np.clip(image, 0, 255).astype(np.uint8))
    draw = ImageDraw.Draw(canvas)
    if flock.falcon is not None:
        fp = flock.falcon[0] - flock.position.mean(0) * np.array([1, 1, 0])
        fx = width / 2 + (fp[0] * ca - fp[1] * sa) * scale
        fy = height * 0.98 + (-fp[2] * math.cos(elevation) + (fp[0] * sa + fp[1] * ca) * math.sin(elevation)) * scale
        if 0 <= fx < width and 0 <= fy < height:
            draw.line([(fx - 9, fy + 4), (fx, fy - 2), (fx + 9, fy + 4)], fill=(235, 70, 80), width=3)
            draw.text((fx + 12, fy - 10), "OOM", font=font(11), fill=(235, 90, 95))
    if highlight is not None:
        mine = flock.owner == highlight
        cx, cy = sx[mine].mean(), sy[mine].min() - 12
        name = PROCESSES[highlight][0]
        draw.text((cx, cy), f"{name}  {mine.sum()} threads", font=font(13), fill=(240, 236, 228), anchor="mm")
    draw.rectangle([0, height - 26, width, height], fill=(18, 18, 26))
    draw.text((8, height - 20), status, font=font(12), fill=(200, 205, 215))
    return canvas


def main(outdir):
    os.makedirs(os.path.join(outdir, "murm-frames"), exist_ok=True)
    flock = Flock()
    for _ in range(260):
        flock.step()
    print("warm")
    frames = []
    total = 240
    for i in range(total):
        if i == 140:
            flock.release_falcon()
        for _ in range(2):
            flock.step()
        azimuth = 0.4 + i * 0.004
        highlight = 0 if 40 <= i < 120 else None
        status = f"ISOTOP / MURMURATION   {len(flock.owner)} threads   7 topological neighbours"
        if highlight is not None:
            status += "   hover: browser-512"
        if i >= 140:
            status += "   memory pressure: falcon"
        image = render(flock, 800, 460, azimuth, highlight, status)
        if i in (20, 90, 160):
            image.save(os.path.join(outdir, f"murmuration-{i}.png"))
        frames.append(image.convert("P", palette=Image.ADAPTIVE, colors=200))
        if i % 50 == 0:
            print("frame", i)
        image.save(os.path.join(outdir, "murm-frames", f"{i:04d}.png"))
    frames[0].save(os.path.join(outdir, "murmuration.gif"), save_all=True, append_images=frames[1:],
                   duration=60, loop=0, optimize=True)
    print("done")


if __name__ == "__main__":
    main(sys.argv[1])
