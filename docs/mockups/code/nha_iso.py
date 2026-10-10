"""Small deferred iso rasterizer shared by the necropolis and hive mockups.

Polygons are painted back to front into an integer id buffer (painter's algorithm, like
isotop's CPU path), then every pixel recovers its world position from its polygon's plane,
so materials can be shaded, textured and fogged per pixel. Render at 2x and downsample.

Projection matches isostyle.iso: +x runs right-down, +y left-down, +z up, 2:1.
"""
import math

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

VIEW = np.array([1.0, 1.0, 1.0]) / math.sqrt(3.0)  # towards the viewer


class Iso:
    def __init__(self, width, height, scale, ox, oy):
        self.w, self.h, self.s, self.ox, self.oy = width, height, scale, ox, oy

    def proj(self, p):
        p = np.asarray(p, np.float64)
        x, y, z = p[..., 0], p[..., 1], p[..., 2]
        return np.stack([self.ox + (x - y) * self.s, self.oy + (x + y) * self.s * 0.5 - z * self.s], -1)


class Scene:
    def __init__(self):
        self.items = []

    def poly(self, pts, color, mat, key, aux=0.0):
        pts = np.asarray(pts, np.float64)
        self.items.append((key, len(self.items), pts, np.asarray(color, np.float32), int(mat), float(aux)))

    def box(self, x0, x1, y0, y1, z0, z1, color, mat, key=None, aux=0.0, top=True, fx=True, fy=True):
        """Axis-aligned box: draws the three faces that face the viewer (+x, +y, +z)."""
        if key is None:
            key = (x0 + x1) * 0.5 + (y0 + y1) * 0.5
        if fy:
            self.poly([(x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)], color, mat, key, aux)
        if fx:
            self.poly([(x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1)], color, mat, key, aux)
        if top:
            self.poly([(x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)], color, mat, key, aux)


class GBuffer:
    pass


def raster(scene, iso):
    items = sorted(scene.items, key=lambda it: (it[0], it[1]))
    n = len(items)
    img = Image.new("I", (iso.w, iso.h), -1)
    draw = ImageDraw.Draw(img)
    normals = np.zeros((n, 3), np.float64)
    minv = np.zeros((n, 3, 3), np.float64)
    dist = np.zeros(n, np.float64)
    colors = np.zeros((n, 3), np.float32)
    mats = np.zeros(n, np.int32)
    auxs = np.zeros(n, np.float32)
    cents = np.zeros((n, 3), np.float64)
    for i, (key, _, pts, col, mat, aux) in enumerate(items):
        sp = iso.proj(pts)
        draw.polygon([(float(a), float(b)) for a, b in sp], fill=i)
        c = pts.mean(0)
        nrm = np.zeros(3)
        for k in range(len(pts)):  # Newell's method, robust for any planar polygon
            a, b = pts[k], pts[(k + 1) % len(pts)]
            nrm += np.array([(a[1] - b[1]) * (a[2] + b[2]), (a[2] - b[2]) * (a[0] + b[0]),
                             (a[0] - b[0]) * (a[1] + b[1])])
        ln = np.linalg.norm(nrm)
        nrm = nrm / ln if ln > 1e-12 else np.array([0.0, 0.0, 1.0])
        if nrm @ VIEW < 0:
            nrm = -nrm
        normals[i] = nrm
        dist[i] = nrm @ c
        m = np.array([[1.0, -1.0, 0.0], [0.5, 0.5, -1.0], nrm])
        if abs(np.linalg.det(m)) > 1e-6:
            minv[i] = np.linalg.inv(m)
        colors[i], mats[i], auxs[i], cents[i] = col, mat, aux, c
    ids = np.asarray(img, np.int32)
    g = GBuffer()
    g.ids = ids
    g.valid = ids >= 0
    idc = np.where(g.valid, ids, 0)
    yy, xx = np.mgrid[0:iso.h, 0:iso.w].astype(np.float64)
    u = (xx + 0.5 - iso.ox) / iso.s
    v = (yy + 0.5 - iso.oy) / iso.s
    d = dist[idc]
    world = np.empty((iso.h, iso.w, 3), np.float32)
    for r in range(3):
        world[..., r] = minv[idc, r, 0] * u + minv[idc, r, 1] * v + minv[idc, r, 2] * d
    g.world = world
    g.normal = normals[idc].astype(np.float32)
    g.color = colors[idc]
    g.mat = np.where(g.valid, mats[idc], -1)
    g.aux = auxs[idc]
    g.u, g.v = u, v
    g.n = n
    return g


def noise_tex(size, sigmas, seed, weights=None):
    """Tileable fractal noise in 0..1."""
    rng = np.random.default_rng(seed)
    out = np.zeros((size, size), np.float32)
    weights = weights or [1.0 / (i + 1) for i in range(len(sigmas))]
    for sg, wt in zip(sigmas, weights):
        f = ndimage.gaussian_filter(rng.standard_normal((size, size)).astype(np.float32), sg, mode="wrap")
        f /= f.std() + 1e-9
        out += wt * f
    out -= out.min()
    out /= out.max() + 1e-9
    return out


def sample(tex, a, b):
    """Bilinear wrap sampling of tex at texel coords (a=col, b=row)."""
    return ndimage.map_coordinates(tex, [b.ravel(), a.ravel()], order=1, mode="grid-wrap").reshape(a.shape)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def downsample(canvas, factor, size):
    img = Image.fromarray(np.clip(canvas, 0, 255).astype(np.uint8))
    return img.resize(size, Image.LANCZOS)
