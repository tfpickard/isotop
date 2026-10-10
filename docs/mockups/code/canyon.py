"""Mockup of isotop's canyon view: a sunlit desert heightfield shaped by the machine's history.

Real simulation: droplet hydraulic erosion (Beyer 2015, vectorised in batches) on a 256 x 256
heightfield, with uplift proportional to memory at each process's plot and slow relaxation
toward base level. Rain is CPU time, conserved: one droplet per 10 ms of CPU, released over
the plot of the process that spent it. Deposited sediment remembers the colour mix of the
kinds of the droplets that carried it. Lakes are depressions filled by a priority flood.
Strata colours and hardness are decorative.

Rendered as an isometric block with a front-to-back column (painter's) rasteriser, so cliffs
show banded walls and the cut faces of the block show the strata.

usage: python3 -I canyon.py [still|anim|test]
"""
import heapq
import math
import os
import sys
import time

sys.path.insert(0, "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto")
import isostyle as S  # noqa: E402

import cv2  # noqa: E402
import numpy as np  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402
from scipy import ndimage  # noqa: E402

N = 256
ZS = 70.0                       # display height in cells per unit of sim height
QUANTUM = 0.010                 # CPU seconds per droplet

# Beyer 2015 / Lague parameters
INERTIA = 0.05
CAP = 4.0
MIN_CAP = 0.0008
ERODE = 0.15
DEPOSIT = 0.3
EVAP = 0.01
GRAV = 4.0
LIFE = 70
RADIUS = 3

GIB = 1.0

# name, kind, memory GiB, average CPU share (cores) over the demo day, plot (x, y) in cells
PROCS = [
    ("postgres-41", "system", 3.1, 0.006, (72, 62)),
    ("browser-512", "session", 2.4, 0.045, (160, 40)),
    ("compiler-90", "session", 0.9, 0.150, (106, 112)),
    ("language-server-71", "session", 1.5, 0.020, (36, 150)),
    ("redis-207", "container", 1.6, 0.003, (190, 170)),
    ("worker-134", "container", 0.6, 0.040, (188, 70)),
    ("nginx-128", "system", 0.2, 0.012, (30, 96)),
    ("journald-310", "system", 0.15, 0.004, (104, 18)),
    ("pipewire-95", "session", 0.06, 0.010, (222, 214)),
    ("systemd-1", "system", 0.05, 0.002, (24, 26)),
    ("shell-170", "session", 0.01, 0.001, (122, 222)),
]


def minor_procs(rng, n=181):
    kinds = ["system", "session", "container", "kernel"]
    out = []
    for i in range(n):
        k = kinds[rng.choice(4, p=[0.3, 0.3, 0.15, 0.25])]
        out.append((f"p{i}", k, float(rng.lognormal(-4.5, 1.0)), float(rng.lognormal(-7.4, 0.8)),
                    (float(rng.uniform(12, N - 12)), float(rng.uniform(12, N - 12)))))
    return out


# ---------------------------------------------------------------- noise and strata
def fbm(n, rng, octaves=6, base=4):
    out = np.zeros((n, n), np.float32)
    amp, tot = 1.0, 0.0
    for o in range(octaves):
        k = base * 2 ** o
        g = rng.standard_normal((k + 3, k + 3)).astype(np.float32)
        up = cv2.resize(g, (n + int(n / k) * 3, n + int(n / k) * 3), interpolation=cv2.INTER_CUBIC)
        out += amp * up[:n, :n]
        tot += amp
        amp *= 0.5
    return out / tot


STRATA_COLS = np.float32([
    (168, 74, 48), (196, 112, 68), (226, 196, 150), (140, 58, 44), (204, 142, 78),
    (182, 92, 58), (232, 208, 168), (156, 70, 52), (210, 128, 74), (120, 54, 46),
])
STRATA_HARD = np.float32([0.9, 1.1, 0.35, 1.0, 1.2, 0.9, 0.35, 1.0, 1.1, 0.8])


def make_bands(rng, zmax=80.0):
    edges, cols, hard = [-20.0], [], []
    z = -20.0
    i = 0
    while z < zmax:
        th = float(rng.uniform(1.4, 4.2))
        k = int(rng.integers(len(STRATA_COLS)))
        if i % 4 == 2:
            k, th = 2 if rng.random() < 0.5 else 6, float(rng.uniform(1.0, 2.0))   # pale caprock
        z += th
        edges.append(z)
        cols.append(STRATA_COLS[k] * float(rng.uniform(0.92, 1.06)))
        hard.append(STRATA_HARD[k])
        i += 1
    return np.float32(edges), np.float32(cols), np.float32(hard)


# ---------------------------------------------------------------- the land
class Canyon:
    def __init__(self, seed=4):
        rng = np.random.default_rng(seed)
        self.rng = rng
        yy, xx = np.mgrid[0:N, 0:N].astype(np.float32)
        self.yy, self.xx = yy, xx
        # A plateau at the back, an escarpment, and a desert floor draining to the front corner.
        line = (xx + yy) / N - 1.02 + 0.16 * fbm(N, rng, 3, 2)
        step = 1 / (1 + np.exp(line * 16))
        plateau = 0.44 + 0.05 * fbm(N, rng, 4, 3)
        floor = 0.10 + 0.08 * (1 - (xx + yy) / (2 * N)) + 0.02 * fbm(N, rng, 3, 4)
        base = floor + (plateau - floor) * step + 0.008 * fbm(N, rng, 2, 24)
        # Two closed basins on the floor (playas) where runoff can pond.
        for bx, by, br, bd in ((228, 146, 15, 0.34), (146, 228, 15, 0.34)):
            base -= bd * np.exp(-((xx - bx) ** 2 + (yy - by) ** 2) / (2 * br * br))
        self.base = base.astype(np.float32)
        self.h = self.base.copy()
        self.h0 = self.base.copy()
        self.procs = PROCS + minor_procs(rng)
        self.kcol = {k: np.float32(v) for k, v in S.KIND.items()}
        # Uplift footprints: flat-topped (mesa) bumps.
        self.uplift = np.zeros((N, N), np.float32)
        self.cap = np.zeros((N, N), np.float32)
        for name, kind, mem, cpu, (px, py) in self.procs:
            if mem < 0.1:
                continue
            R = 9 + 9 * math.sqrt(mem)
            r = np.sqrt((xx - px) ** 2 + (yy - py) ** 2) / R
            fp = np.exp(-r ** 10)
            self.uplift += np.float32(0.17 * mem ** 0.75) * fp
            if mem >= 1.0:
                self.cap = np.maximum(self.cap, np.clip(fp * 1.6, 0, 1))
        self.edges, self.bcols, self.bhard = make_bands(np.random.default_rng(seed + 1))
        self.warp = 1.6 * fbm(N, np.random.default_rng(seed + 2), 3, 3)
        self.sed_w = np.zeros(N * N, np.float32)
        self.sed_c = np.zeros((N * N, 3), np.float32)
        self.flow = np.zeros(N * N, np.float32)
        self.pond = np.zeros(N * N, np.float32)       # water that ended its run here
        self.droplets = 0
        self.carry = np.zeros(len(self.procs))        # CPU-seconds remainder per process
        self.cpu_s = 0.0
        # Erosion brush (radius 3, linear falloff), as flat-index offsets with weights.
        offs = []
        for dy in range(-RADIUS, RADIUS + 1):
            for dx in range(-RADIUS, RADIUS + 1):
                d = math.hypot(dx, dy)
                if d < RADIUS:
                    offs.append((dy, dx, RADIUS - d))
        w = np.float32([o[2] for o in offs])
        self.brush_dy = np.int32([o[0] for o in offs])
        self.brush_dx = np.int32([o[1] for o in offs])
        self.brush_w = w / w.sum()
        self.lake = np.zeros((N, N), np.float32)
        # Caprock: resistant layer on the mesas (decorative hardness, like the strata).
        self.soft = (1 - 0.85 * self.cap).reshape(-1).astype(np.float32)
        self.uplift_done = 0.0

    def hardness(self, hcell):
        z = hcell * ZS
        k = np.clip(np.searchsorted(self.edges, z) - 1, 0, len(self.bhard) - 1)
        return self.bhard[k]

    # ---- rain bookkeeping: integer droplets, remainder carried forward
    def release(self, machine_seconds, cap=None):
        """Turn CPU time into droplets per process. Returns arrays of start x, y and kind colour."""
        xs, ys, cols = [], [], []
        total = 0
        for i, (name, kind, mem, cpu, (px, py)) in enumerate(self.procs):
            self.carry[i] += cpu * machine_seconds
            n = int(self.carry[i] / QUANTUM)
            if n <= 0:
                continue
            self.carry[i] -= n * QUANTUM
            total += n
            rad = 12.0 if name == "compiler-90" else (18.0 if cpu > 0.0015 else 14.0)
            xs.append(px + self.rng.normal(0, rad, n))
            ys.append(py + self.rng.normal(0, rad, n))
            cols.append(np.repeat(self.kcol[kind][None], n, 0))
        self.cpu_s += machine_seconds * sum(p[3] for p in self.procs)
        if not xs:
            return None
        return (np.clip(np.concatenate(xs), 1, N - 2.01), np.clip(np.concatenate(ys), 1, N - 2.01),
                np.concatenate(cols))

    def erode(self, x, y, col, batch=2048):
        perm = self.rng.permutation(len(x))
        x, y, col = x[perm], y[perm], col[perm]
        for s in range(0, len(x), batch):
            self._erode_batch(x[s:s + batch].astype(np.float32), y[s:s + batch].astype(np.float32),
                              col[s:s + batch])
        self.droplets += len(x)

    def _erode_batch(self, px, py, col):
        h = self.h.reshape(-1)
        B = len(px)
        dx = np.zeros(B, np.float32)
        dy = np.zeros(B, np.float32)
        speed = np.ones(B, np.float32)
        water = np.ones(B, np.float32)
        sed = np.zeros(B, np.float32)
        alive = np.ones(B, bool)
        eroded = np.zeros(N * N, np.float32)
        dep_w = np.zeros(N * N, np.float32)
        dep_c = np.zeros((N * N, 3), np.float32)
        for step in range(LIFE):
            idx = np.nonzero(alive)[0]
            if idx.size == 0:
                break
            x, y = px[idx], py[idx]
            ix = x.astype(np.int32)
            iy = y.astype(np.int32)
            fx, fy = x - ix, y - iy
            c = iy * N + ix
            h00, h10, h01, h11 = h[c], h[c + 1], h[c + N], h[c + N + 1]
            gx = (h10 - h00) * (1 - fy) + (h11 - h01) * fy
            gy = (h01 - h00) * (1 - fx) + (h11 - h10) * fx
            hh = h00 * (1 - fx) * (1 - fy) + h10 * fx * (1 - fy) + h01 * (1 - fx) * fy + h11 * fx * fy
            ndx = dx[idx] * INERTIA - gx * (1 - INERTIA)
            ndy = dy[idx] * INERTIA - gy * (1 - INERTIA)
            ln = np.sqrt(ndx * ndx + ndy * ndy)
            ok = ln > 1e-9
            ndx = np.where(ok, ndx / np.maximum(ln, 1e-9), 0)
            ndy = np.where(ok, ndy / np.maximum(ln, 1e-9), 0)
            nx, ny = x + ndx, y + ndy
            self.flow += np.bincount(c, weights=water[idx], minlength=N * N).astype(np.float32)
            inside = ok & (nx >= 0) & (nx < N - 1.01) & (ny >= 0) & (ny < N - 1.01)
            # New height (only meaningful inside).
            nxc = np.clip(nx, 0, N - 1.001)
            nyc = np.clip(ny, 0, N - 1.001)
            jx, jy = nxc.astype(np.int32), nyc.astype(np.int32)
            gfx, gfy = nxc - jx, nyc - jy
            d = jy * N + jx
            nh = (h[d] * (1 - gfx) * (1 - gfy) + h[d + 1] * gfx * (1 - gfy) + h[d + N] * (1 - gfx) * gfy
                  + h[d + N + 1] * gfx * gfy)
            dh = nh - hh
            s = sed[idx]
            # Parallel droplets sharing a cell split the work (keeps the batched scheme stable).
            dens = np.bincount(c, minlength=N * N)[c].astype(np.float32)
            share = 1.0 / dens ** 0.5
            capa = np.maximum(-dh * speed[idx] * water[idx] * CAP, MIN_CAP)
            dep_mask = (s > capa) | (dh > 0)
            amount_dep = np.where(dh > 0, np.minimum(dh, s), (s - capa) * DEPOSIT)
            amount_dep = np.where(dep_mask, amount_dep * share, 0.0)
            # Droplets leaving the map or dying on their last step drop what they carry here.
            last = (~inside) | (step == LIFE - 1)
            amount_dep = np.where(~inside, 0.0, amount_dep)
            amount_ero = np.where(dep_mask | last, 0.0,
                                  np.minimum((capa - s) * ERODE * self.hardness(hh) * self.soft[c], -dh) * share)
            s = s - amount_dep + amount_ero
            sed[idx] = s
            # Deposition: bilinear onto the four corners of the old cell.
            if amount_dep.any():
                wts = [(1 - fx) * (1 - fy), fx * (1 - fy), (1 - fx) * fy, fx * fy]
                cells = np.concatenate([c, c + 1, c + N, c + N + 1])
                a = np.concatenate([amount_dep * ww for ww in wts])
                step_dep = np.minimum(np.bincount(cells, weights=a, minlength=N * N).astype(np.float32), 0.004)
                h += step_dep
                dep_w += step_dep
                cc = np.tile(col[idx], (4, 1))
                for k in range(3):
                    dep_c[:, k] += np.bincount(cells, weights=a * cc[:, k], minlength=N * N).astype(np.float32)
            if amount_ero.any():
                bx = np.clip(ix[:, None] + self.brush_dx[None], 0, N - 1)
                by = np.clip(iy[:, None] + self.brush_dy[None], 0, N - 1)
                bc = (by * N + bx).reshape(-1)
                bw = (amount_ero[:, None] * self.brush_w[None]).reshape(-1)
                step_ero = np.minimum(np.bincount(bc, weights=bw, minlength=N * N).astype(np.float32), 0.004)
                h -= step_ero
                eroded += step_ero
            speed[idx] = np.sqrt(np.maximum(speed[idx] ** 2 + dh * GRAV, 0))
            water[idx] *= (1 - EVAP)
            dx[idx], dy[idx] = ndx, ndy
            px[idx], py[idx] = nx, ny
            alive[idx] = inside & (step < LIFE - 1)
            if step == LIFE - 1:
                fin = inside
                cell = (np.clip(ny[fin], 0, N - 1).astype(np.int32) * N + np.clip(nx[fin], 0, N - 1).astype(np.int32))
                self.pond += np.bincount(cell, weights=water[idx][fin], minlength=N * N).astype(np.float32)
        # Sediment colour bookkeeping: erosion strips colour, deposition adds the carriers' mix.
        frac = np.clip(eroded / np.maximum(self.sed_w, 1e-6), 0, 1)
        self.sed_w *= (1 - frac)
        self.sed_c *= (1 - frac)[:, None]
        self.sed_w += dep_w
        self.sed_c += dep_c

    def tectonics(self, dt_hours):
        """Uplift ~ memory at each plot, relaxation toward base level (time constant ~ 8 h)."""
        self.h += self.uplift * np.float32(dt_hours / 6.2)
        self.h -= (self.h - self.base) * np.float32(1 - math.exp(-dt_hours / 8.0)) * 0.3
        np.maximum(self.h, -0.1, out=self.h)
        self.uplift_done += dt_hours

    def lakes(self):
        """Priority flood from the block edges; depressions deeper than a hair hold water."""
        h = self.h
        filled = np.full((N, N), np.inf, np.float32)
        seen = np.zeros((N, N), bool)
        heap = []
        for i in range(N):
            for (y, x) in ((0, i), (N - 1, i), (i, 0), (i, N - 1)):
                if not seen[y, x]:
                    seen[y, x] = True
                    filled[y, x] = h[y, x]
                    heap.append((float(h[y, x]), y, x))
        heapq.heapify(heap)
        hl = h.tolist()
        fl = filled
        while heap:
            v, y, x = heapq.heappop(heap)
            for ny, nx in ((y - 1, x), (y + 1, x), (y, x - 1), (y, x + 1)):
                if 0 <= ny < N and 0 <= nx < N and not seen[ny, nx]:
                    seen[ny, nx] = True
                    nv = max(v, hl[ny][nx])
                    fl[ny, nx] = nv
                    heapq.heappush(heap, (nv, ny, nx))
        depth = fl - h
        # Constant evaporation: only basins that collect enough water keep a lake.
        lab, n = ndimage.label(depth > 0.0015)
        if n:
            size = ndimage.sum(np.ones_like(depth), lab, index=np.arange(1, n + 1))
            keep = np.zeros(n + 1, bool)
            keep[1:] = size >= 20
            depth = np.where(keep[lab], depth, 0)
        # Ponds: where spent droplets collect on flat ground, net of evaporation.
        pond = ndimage.gaussian_filter(self.pond.reshape(N, N), 2.0)
        gy, gx = np.gradient(h)
        flat = ndimage.gaussian_filter(np.hypot(gx, gy), 1.0) < 0.0022
        thr = np.percentile(pond, 96.5)
        pd = 0.012 * np.clip(pond / max(thr, 1e-6) - 1.0, 0, 1.5) * flat
        lab2, n2 = ndimage.label(pd > 0)
        if n2:
            size = ndimage.sum(np.ones_like(pd), lab2, index=np.arange(1, n2 + 1))
            keep = np.zeros(n2 + 1, bool)
            keep[1:] = size >= 25
            pd = np.where(keep[lab2], pd, 0)
        self.lake = np.maximum(depth, ndimage.gaussian_filter(pd, 0.8)).astype(np.float32)
        return self.lake


# ---------------------------------------------------------------- shading
LIGHT = np.float32([-0.45, 0.72, 0.55])
LIGHT = LIGHT / np.linalg.norm(LIGHT)
SAND = np.float32([228, 190, 140])
OCHRE = np.float32([212, 150, 92])
WATER_DEEP = np.float32([22, 70, 84])
WATER_SHALLOW = np.float32([64, 124, 120])


def strata_colour(cy, z, warp):
    zz = z + warp
    k = np.clip(np.searchsorted(cy.edges, zz) - 1, 0, len(cy.bcols) - 1)
    return cy.bcols[k]


def surface(cy, R):
    """Colour, height (display cells) and normals of the top surface at R x R resolution."""
    lake = cy.lake
    hs = (cy.h + lake) * ZS
    Hr = cv2.resize(hs, (R, R), interpolation=cv2.INTER_CUBIC)
    hr = cv2.resize(cy.h * ZS, (R, R), interpolation=cv2.INTER_CUBIC)
    lk = cv2.resize(lake * ZS, (R, R), interpolation=cv2.INTER_LINEAR)
    k = R / N
    gy, gx = np.gradient(Hr, 1.0 / k)
    nrm = np.stack([-gx, -gy, np.ones_like(Hr)], -1)
    nrm /= np.linalg.norm(nrm, axis=-1, keepdims=True)
    slope = 1 - nrm[..., 2]
    diffuse = np.clip(nrm @ LIGHT, 0, 1)
    shadow = cast_shadow(Hr, k)
    # Cavity: valleys a touch darker.
    cav = np.clip((cv2.GaussianBlur(Hr, (0, 0), 6 * k) - Hr) / 4.0, -1, 1)
    warp = cv2.resize(cy.warp, (R, R), interpolation=cv2.INTER_CUBIC)
    strata = strata_colour(cy, hr, warp)
    flat = (np.clip(1 - slope * 7.0, 0, 1) ** 1.5)[..., None]
    low = np.clip((hr - 14) / 18, 0, 1)[..., None]
    mott = cv2.resize(fbm(N, np.random.default_rng(21), 3, 5), (R, R), interpolation=cv2.INTER_CUBIC)[..., None]
    ground = SAND * (1 - low) + OCHRE * low
    ground = ground * (1 - 0.35 * np.clip(mott, 0, 1)) + np.float32([196, 112, 74]) * 0.35 * np.clip(mott, 0, 1)
    base = strata * (1 - flat * 0.88) + ground * (flat * 0.88)
    # Sediment: alluvial fans tinted by the kinds that carried them.
    sw = cy.sed_w.reshape(N, N)
    sc = cy.sed_c.reshape(N, N, 3) / np.maximum(sw, 1e-6)[..., None]
    amt = np.clip((sw - 0.008) / 0.05, 0, 1) ** 0.8
    amt = cv2.resize(amt, (R, R), interpolation=cv2.INTER_LINEAR)[..., None]
    scol = cv2.resize(sc, (R, R), interpolation=cv2.INTER_LINEAR)
    fan = SAND * 0.45 + scol * 0.55
    base = base * (1 - amt * 0.55) + fan * (amt * 0.55)
    # Varnish speckle.
    speck = cv2.resize(fbm(N, np.random.default_rng(9), 2, 64), (R, R), interpolation=cv2.INTER_LINEAR)
    base = base * (0.94 + 0.10 * speck[..., None])
    light = 0.38 + 0.80 * diffuse * (0.35 + 0.65 * shadow)
    light = light * (1 - 0.18 * np.clip(cav, 0, 1))
    col = base * light[..., None]
    # Rivers: where many droplets ran recently.
    flow = cy.flow.reshape(N, N)
    f = np.clip(flow / (np.percentile(flow, 99.7) + 1e-6), 0, 1) ** 1.6
    f = cv2.resize(f, (R, R), interpolation=cv2.INTER_LINEAR)[..., None]
    col = col * (1 - 0.62 * f) + np.float32([70, 150, 156]) * light[..., None] * 0.95 * (0.62 * f)
    # Lakes.
    wet = np.clip(lk / 0.6, 0, 1)[..., None]
    deep = np.clip(lk / 4.0, 0, 1)[..., None]
    water = WATER_SHALLOW * (1 - deep) + WATER_DEEP * deep
    water = water * (0.85 + 0.25 * shadow[..., None]) + 22 * np.float32([0.6, 0.8, 1.0])
    col = col * (1 - wet) + water * wet
    hn = np.stack([-gx, -gy], -1)
    return col.astype(np.float32), Hr, hn, warp


def cast_shadow(H, k, steps=90):
    lx, ly, lz = LIGHT
    hl = math.hypot(lx, ly)
    dx, dy = lx / hl, ly / hl
    rise = lz / hl / k            # display height gained per pixel step toward the light
    R = H.shape[0]
    yy, xx = np.mgrid[0:R, 0:R].astype(np.float32)
    lit = np.ones_like(H)
    stride = 2.0
    for s in range(1, steps):
        d = s * stride
        sx, sy = xx + dx * d, yy + dy * d
        hh = cv2.remap(H, sx, sy, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
        over = hh - (H + d * rise * k)
        lit = np.minimum(lit, np.clip(1 - over / 1.2, 0, 1))
    return cv2.GaussianBlur(lit, (0, 0), 0.8)


# ---------------------------------------------------------------- isometric column rasteriser
ZMIN = -16.0     # block base below sea level (display cells)


def rasterise(cy, w, h, ss, view):
    W, H = w * ss, h * ss
    s, ox, oy = view["s"] * ss, view["ox"] * ss, view["oy"] * ss
    R = 2 * N
    col, Hr, hn, warp = surface(cy, R)
    k = R / N
    # Front-to-back march along each screen column: c = x - y fixed, t = x + y decreasing.
    u = np.arange(W, dtype=np.float32) + 0.5
    c = (u - ox) / s
    dt = min(0.5, 1.4 / s)
    ts = np.arange(2 * (N - 1), -dt, -dt, dtype=np.float32)
    X = (ts[:, None] + c[None]) / 2
    Y = (ts[:, None] - c[None]) / 2
    valid = (X >= 0) & (X <= N - 1) & (Y >= 0) & (Y <= N - 1)
    Hs = cv2.remap(Hr, (X * k).astype(np.float32), (Y * k).astype(np.float32), cv2.INTER_LINEAR,
                   borderMode=cv2.BORDER_REPLICATE)
    top = oy + ts[:, None] * s / 2 - Hs * s
    top = np.where(valid, top, np.inf)
    m = np.minimum.accumulate(top, axis=0)
    rows = np.arange(H, dtype=np.float32) + 0.5
    idx = np.empty((H, W), np.int32)
    negm = -m
    for j in range(W):
        idx[:, j] = np.searchsorted(negm[:, j], -rows, side="left")
    D = len(ts)
    cover = idx < D
    idc = np.minimum(idx, D - 1)
    jj = np.broadcast_to(np.arange(W)[None], (H, W))
    tx = X[idc, jj]
    ty = Y[idc, jj]
    tt = ts[idc]
    ztop = Hs[idc, jj]
    zpix = (oy + tt * s / 2 - rows[:, None]) / s
    bottom_ok = zpix >= ZMIN
    cover &= bottom_ok & valid[idc, jj]
    on_top = zpix >= ztop - 0.9 / s * 2
    img = np.zeros((H, W, 3), np.float32)
    mx, my = (tx * k).astype(np.float32), (ty * k).astype(np.float32)
    topcol = cv2.remap(col, mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    # Walls: strata at the pixel's own height, shaded by which way the wall faces.
    wv = cv2.remap(warp, mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    wall = strata_colour(cy, zpix, wv)
    gx = cv2.remap(np.ascontiguousarray(hn[..., 0]), mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    gy = cv2.remap(np.ascontiguousarray(hn[..., 1]), mx, my, cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    edge_x = tx > N - 1 - 0.75
    edge_y = ty > N - 1 - 0.75
    nxw = np.where(edge_x, 1.0, np.where(edge_y, 0.0, gx))
    nyw = np.where(edge_y, 1.0, np.where(edge_x, 0.0, gy))
    ln = np.sqrt(nxw ** 2 + nyw ** 2) + 1e-6
    lam = (nxw / ln) * LIGHT[0] + (nyw / ln) * LIGHT[1]
    wshade = 0.42 + 0.62 * np.clip(lam, 0, 1)
    # Deeper in the wall reads darker; the bedrock below sea level is plain and dark.
    depth = np.clip((ztop - zpix) / 30, 0, 1)
    wshade = wshade * (1 - 0.25 * depth)
    wall = wall * wshade[..., None]
    below = zpix < 0
    wall = np.where(below[..., None], wall * 0.55, wall)
    img = np.where(on_top[..., None], topcol, wall)
    return img, cover


def make_view(w, h, band):
    scene_h = h - band
    s = w * 0.86 / (2 * (N - 1))
    span_y = (N - 1) * s + 30 * s       # diamond height + relief allowance
    ox = w * 0.5
    oy = (scene_h - (N - 1) * s) * 0.5 + 10 * s
    return {"s": s, "ox": ox, "oy": oy}


def project(view, x, y, z):
    s = view["s"]
    return view["ox"] + (x - y) * s, view["oy"] + (x + y) * s / 2 - z * s


def render(cy, w, h, lines, labels, ss=2):
    size = max(11, w // 115)
    band = int(size * 1.45) * len(lines) + 10
    view = make_view(w, h, band)
    sky = S.sky(w, h)
    big_sky = cv2.resize(sky, (w * ss, h * ss), interpolation=cv2.INTER_LINEAR)
    img, cover = rasterise(cy, w, h, ss, view)
    # Soft drop shadow under the block.
    sh = np.zeros((h * ss, w * ss), np.float32)
    s = view["s"] * ss
    pts = [project(view, x, y, ZMIN) for x, y in ((0, 0), (N - 1, 0), (N - 1, N - 1), (0, N - 1))]
    pts = np.float32(pts) * ss
    pts[:, 1] += 14 * ss
    cv2.fillConvexPoly(sh, np.int32(pts * 16), 1.0, cv2.LINE_AA, 4)
    sh = cv2.GaussianBlur(sh, (0, 0), w * ss / 60)
    big_sky *= (1 - 0.55 * sh)[..., None]
    out = np.where(cover[..., None], img, big_sky)
    # Haze: farther (higher on screen) parts of the land pick up a little sky colour.
    out = cv2.resize(out, (w, h), interpolation=cv2.INTER_AREA)
    pil = S.to_image(out)
    draw = ImageDraw.Draw(pil)
    fs = max(11, int(w / 112))
    for text, (x, y, z), (dx, dy) in labels:
        sx, sy = project(view, x, y, z)
        tx, ty = sx + dx * fs, sy + dy * fs
        draw.line([(sx, sy), (tx, ty + (fs * 0.55 if dy < 0 else -fs * 0.55))], fill=(240, 236, 228), width=1)
        draw.ellipse([sx - 2, sy - 2, sx + 2, sy + 2], fill=(250, 248, 240), outline=(0, 0, 0))
        label(draw, tx, ty, text, fs)
    S.status(pil, lines)
    return pil


def label(draw, x, y, text, size, fill=S.TEXT):
    f = S.font(size)
    draw.text((x + 1, y + 1), text, font=f, fill=(0, 0, 0), anchor="mm", stroke_width=2, stroke_fill=(0, 0, 0))
    draw.text((x, y), text, font=f, fill=fill, anchor="mm", stroke_width=1, stroke_fill=(10, 8, 6))


# ---------------------------------------------------------------- labels and status
LEGEND = ("uplift = memory at each plot | rain = CPU time, 1 droplet per 10 ms | sediment colour = kind of the"
          " rain that carried it | lakes = pits | strata decorative")
KEYS = "Tab next view | g tour | scroll pan | click inspect | / search | c links | Space pause | ? help | q quit"


def fmt_uptime(hours):
    m = int(round(hours * 60))
    return f"{m // 60} h {m % 60:02d} m"


def status_lines(cy, hours, rate, short=False):
    line = (f"ISOTOP / CANYON / DEMO   192 processes | time-lapse x{rate}  uptime {fmt_uptime(hours)}"
            f"  droplets {cy.droplets:,} = {cy.droplets // 100:,} CPU-s | plots keyed by cgroup + name")
    if short:
        return [line.split(" | plots")[0],
                "uplift = memory | rain = CPU, 1 droplet / 10 ms | fan colour = kind | strata decorative"]
    return [line, LEGEND, KEYS]


def feature_labels(cy, which):
    out = []
    for name, kind, mem, cpu, (px, py) in cy.procs[:11]:
        if name not in which:
            continue
        kind_txt, off = which[name]
        if kind_txt in ("mesa", "butte"):
            r = 9 + 9 * math.sqrt(mem)
            sl = (slice(int(py - r * 0.4), int(py + r * 0.4)), slice(int(px - r * 0.4), int(px + r * 0.4)))
            yy, xx = np.unravel_index(np.argmax(cy.h[sl]), cy.h[sl].shape)
            x, y = xx + sl[1].start, yy + sl[0].start
            text = f"{name} {kind_txt} {mem:.1f} GiB"
        else:
            # The deepest cut downstream of the plot.
            cut = cy.h0 - cy.h if kind_txt == "gorge" else cy.sed_w.reshape(N, N)
            r = 60
            y0, y1 = int(max(py - 10, 0)), int(min(py + r, N))
            x0, x1 = int(max(px - 10, 0)), int(min(px + r, N))
            sub = ndimage.gaussian_filter(cut, 2)[y0:y1, x0:x1]
            yy, xx = np.unravel_index(np.argmax(sub), sub.shape)
            x, y = xx + x0, yy + y0
            text = f"{name} {kind_txt}"
        z = (cy.h[int(y), int(x)] + cy.lake[int(y), int(x)]) * ZS
        out.append((text, (x, y, z), off))
    return out


LABELS = {"postgres-41": ("mesa", (-4.0, -3.0)), "browser-512": ("mesa", (4.0, -3.0)),
          "compiler-90": ("gorge", (-5.0, 3.0)), "redis-207": ("butte", (4.0, -2.6)),
          "worker-134": ("fan", (5.0, 2.4))}


def simulate(cy, hours, rate_droplets_per_frame=None, frames=1, on_frame=None):
    """Advance the machine `hours` of demo time over `frames` steps."""
    dt = hours / frames
    for f in range(frames):
        drops = cy.release(dt * 3600.0)
        cy.tectonics(dt)
        if drops is not None:
            cy.erode(*drops)
        cy.flow *= 0.55
        cy.pond *= 0.8
        if on_frame:
            on_frame(f)


def still():
    t0 = time.time()
    cy = Canyon()
    hours = 6.2
    simulate(cy, hours, frames=40)
    # Rivers as they run right now: one more short burst so flow reflects current rain.
    cy.lakes()
    np.savez_compressed(f"{S.OUT}/canyon-state.npz", h=cy.h, pond=cy.pond, lake=cy.lake, sw=cy.sed_w)
    print("sim", round(time.time() - t0, 1), "droplets", cy.droplets, "lake cells", int((cy.lake > 0).sum()), flush=True)
    img = render(cy, 1600, 900, status_lines(cy, hours, 600), feature_labels(cy, LABELS), ss=2)
    print(S.save_still(img, "canyon"), round(time.time() - t0, 1))


def anim():
    t0 = time.time()
    cy = Canyon()
    frames = []
    total = 120
    hours = 6.2
    simulate(cy, 0.25, frames=4)
    rate = int(round((hours - 0.25) * 3600 / (total / 12)))

    def grab(f):
        hrs = 0.25 + (hours - 0.25) * (f + 1) / total
        if f % 6 == 0 or f == total - 1:
            cy.lakes()
        lab = {k: v for k, v in LABELS.items() if k in ("postgres-41", "compiler-90", "browser-512")}
        labels = feature_labels(cy, lab) if f > 30 else []
        frames.append(render(cy, 960, 540, status_lines(cy, hrs, rate, short=True), labels, ss=2))
        if f % 10 == 0:
            print("frame", f, cy.droplets, round(time.time() - t0, 1), flush=True)

    simulate(cy, hours - 0.25, frames=total, on_frame=grab)
    S.save_frames(frames, "canyon", fps=12)
    import stabgif
    print(stabgif.pack("canyon", fps=12, colors=128, thresh=10), round(time.time() - t0, 1))


def test():
    t0 = time.time()
    cy = Canyon()
    simulate(cy, 6.2, frames=30)
    print("sim", round(time.time() - t0, 1), "droplets", cy.droplets, "h", cy.h.min(), cy.h.max(), flush=True)
    cy.lakes()
    print("lakes", round(time.time() - t0, 1), (cy.lake > 0).sum(), flush=True)
    img = render(cy, 1600, 900, status_lines(cy, 6.2, 600), feature_labels(cy, LABELS), ss=1)
    img.save(f"{S.OUT}/canyon-test.png")
    print("render", round(time.time() - t0, 1))
    np.savez_compressed(f"{S.OUT}/canyon-test.npz", h=cy.h, sw=cy.sed_w, sc=cy.sed_c, flow=cy.flow)


if __name__ == "__main__":
    {"still": still, "anim": anim, "test": test}[sys.argv[1] if len(sys.argv) > 1 else "test"]()
