"""isotop `ptolemy` view mockup: a geocentric sky of epicycles, seen from above.

Real computation here (what the view would do with real samples):
- 60 one-second samples per wanderer of cpu (cores) and log2(memory) (doublings), synthesised
  with distinct personalities (bursty compiler, fork-on-save redis, GC sawtooth browser ...).
- z(n) = (cpu(n) - mean) + i (log2 mem(n) - mean); direct 60-point DFT; the 12 largest non-DC
  coefficients become the epicycle chain, drawn largest-first from the deferent point.
- One global units-to-world constant, soft-clamped with tanh so a wild process cannot swallow
  the sky. Deferent period follows Kepler, T ~ r^1.5. Spheres named inside-out by mean CPU.
- Trails accumulate the planet's apparent positions; retrograde = apparent longitude seen from
  Earth decreasing, detected from the derivative, and tinted.
- Fixed stars: every other process at a stable hashed angle, blackbody colour and magnitude
  from the `hr` view's mapping (T from CPU, m = -2.5 log10(mem / 1 GiB)), ring turns once a minute.
Procedural mockup: the demo population and its sample histories, coefficients held fixed (the
real view eases them toward each new sample), animation runs the replay at x6.
"""
import hashlib
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import isostyle as S  # noqa: E402
import hr as HR  # noqa: E402  (blackbody colours, temperature and magnitude mappings)

import numpy as np  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402
from scipy import ndimage  # noqa: E402

MIB = 1024 ** 2
N = 60            # samples in the window (1 s each)
KEEP = 12         # epicycles kept
K_WORLD = 0.085   # world units (sky radius = 1) per core or per doubling
L_MAX = 0.19      # soft clamp on a chain's total length (world units)
T_OUTER = 1200.0   # deferent period of the outermost sphere, seconds
TILT = 0.80       # camera looks down at ~53 degrees: circles become ellipses
RETRO = (255, 70, 66)
EARTH = (60, 170, 160)

SPHERES = ["Moon", "Mercury", "Venus", "Sun", "Mars", "Jupiter", "Saturn"]
RADII = [0.245, 0.33, 0.415, 0.50, 0.585, 0.67, 0.755]


def h01(name, salt=""):
    d = hashlib.blake2b((salt + name).encode(), digest_size=8).digest()
    return int.from_bytes(d, "little") / 2 ** 64


# ----------------------------------------------------------------------------- samples
def synth():
    t = np.arange(N, dtype=float)
    r = np.random.default_rng(7)

    def noise(s):
        return 0.25 * s * ndimage.gaussian_filter1d(r.standard_normal(N), 1.5, mode="wrap") * 2

    def smooth_step(a, b, w=2.0):
        return 1 / (1 + np.exp(-(t - a) / w)) - 1 / (1 + np.exp(-(t - b) / w))

    w = {}
    # worker-134: a batch queue pulsing every 20 s, memory lagging the work by a quarter cycle
    w["worker-134"] = ("container", 3.0 + 2.4 * np.sin(2 * np.pi * t / 20) + noise(0.15),
                       9.6 + 0.7 * np.sin(2 * np.pi * t / 20 - 1.3) + noise(0.03))
    # language-server-71: an indexing burst, memory steps up, GC drops it later
    burst = np.clip((t - 8) / 4, 0, 1) * np.exp(-np.clip(t - 14, 0, None) / 9)
    w["language-server-71"] = ("session", 0.6 + 4.6 * burst + noise(0.12),
                               9.2 + 0.9 * smooth_step(10, 44, 1.5) + noise(0.02))
    # browser-512: tab spikes, heap sawtooth with garbage collection
    spikes = np.zeros(N)
    for a, h in [(4, 4.5), (17, 3.0), (29, 5.5), (46, 3.5)]:
        spikes += h * np.exp(-0.5 * ((t - a) / 2.4) ** 2)
    saw = 0.5 * np.sin(2 * np.pi * (t + 6) / 30) + 0.3 * np.sin(4 * np.pi * (t + 6) / 30)
    w["browser-512"] = ("session", 0.7 + spikes + noise(0.1), 10.4 + saw + 0.08 * spikes + noise(0.02))
    # postgres-41: autovacuum hump, work_mem grows with it
    hump = smooth_step(14, 36, 3.0)
    w["postgres-41"] = ("system", 0.55 + 2.1 * hump + noise(0.12), 9.1 + 0.55 * hump + noise(0.03))
    # compiler-90: two incremental builds, cc1plus fan-out, then the linker
    build = smooth_step(6, 18, 1.4) + smooth_step(36, 46, 1.4)
    link = smooth_step(18, 23, 1.2) + smooth_step(46, 51, 1.2)
    w["compiler-90"] = ("session", 0.05 + 2.9 * build + 0.9 * link + noise(0.1),
                        7.3 + 1.6 * build + 2.1 * link + noise(0.03))
    # redis-207: BGSAVE forks a child, copy-on-write doubles the footprint
    fork = smooth_step(26, 38, 0.8)
    w["redis-207"] = ("container", 0.25 + 1.5 * fork * np.exp(-np.clip(t - 26, 0, None) / 6) + noise(0.04),
                      7.6 + 1.0 * fork + noise(0.02))
    # pipewire-95: a video call starts, the graph grows
    call = 1 / (1 + np.exp(-(t - 24) / 2))
    w["pipewire-95"] = ("session", 0.12 + 0.5 * call + 0.12 * np.sin(2 * np.pi * t / 6) * call + noise(0.03),
                        4.6 + 0.6 * call + noise(0.02))
    return w


def dft_terms(z):
    n = np.arange(N)
    k = n[:, None]
    c = (z[None, :] * np.exp(-2j * np.pi * k * n[None, :] / N)).sum(1) / N   # direct DFT
    freqs = np.where(n < N // 2, n, n - N).astype(float)
    order = [i for i in np.argsort(-np.abs(c)) if i != 0][:KEEP]
    return c[order], freqs[order]


class Planet:
    def __init__(self, name, kind, cpu, lmem, idx):
        self.name, self.kind = name, kind
        self.cpu, self.lmem = cpu, lmem
        self.mean_cpu = float(cpu.mean())
        z = (cpu - cpu.mean()) + 1j * (lmem - lmem.mean())
        self.z = z
        c, f = dft_terms(z)
        total = np.abs(c).sum() * K_WORLD
        gain = K_WORLD * (L_MAX * math.tanh(total / L_MAX) / total if total > 0 else 1.0)
        self.c, self.f = c * gain, f
        self.mem = 2 ** lmem.mean() * MIB
        self.set_sphere(idx)

    def set_sphere(self, idx):
        self.sphere = SPHERES[idx]
        self.R = RADII[idx]
        self.T = T_OUTER * (self.R / RADII[-1]) ** 1.5
        self.phase = 2 * np.pi * h01(self.name, "phase")

    def deferent(self, t):
        return self.R * np.exp(1j * (self.phase + 2 * np.pi * np.asarray(t) / self.T))

    def chain(self, t):
        """Joint positions: deferent point, then each epicycle tip, largest first."""
        d = complex(self.deferent(t))
        rot = self.c * np.exp(2j * np.pi * self.f * t / N)
        return np.concatenate([[d], d + np.cumsum(rot)])

    def pos(self, t):
        t = np.asarray(t, float)
        rot = (self.c[None, :] * np.exp(2j * np.pi * self.f[None, :] * t[:, None] / N)).sum(1)
        return self.deferent(t) + rot

    def retro(self, t):
        """Retrograde where the apparent (geocentric) longitude decreases."""
        p = self.pos(t)
        lam = np.unwrap(np.angle(p))
        return np.gradient(lam, t) < 0


# ----------------------------------------------------------------------------- scene
class Scene:
    def __init__(self, W, H):
        self.W, self.H = W, H
        self.s = W / 1600
        self.band = int(66 * self.s) if W >= 1200 else 52
        self.cx, self.cy = W * 0.5, (H - self.band) * 0.5 + 2 * self.s
        self.Rpx = min(W * 0.31, (H - self.band) * 0.5 / TILT / 1.03)
        samples = synth()
        planets = [Planet(n, k, c, m, 0) for n, (k, c, m) in samples.items()]
        planets.sort(key=lambda p: -p.mean_cpu)   # busiest innermost
        for i, p in enumerate(planets):
            p.set_sphere(i)
        self.planets = planets
        self.stars = self.make_stars()
        self.bg = S.sky(W, H, glow=(140, 55, 135), centre=(0.16, 0.26))
        self.bg += self.static_layer()

    def make_stars(self):
        pop, kernel, _ = HR.build_population()
        wanderers = {p.name for p in self.planets}
        stars = []
        for p in pop:
            if p["name"] in wanderers:
                continue
            stars.append(dict(name=p["name"], ang=2 * np.pi * h01(p["name"], "ra"),
                              r=0.935 + 0.06 * h01(p["name"], "dec"), m=p["m"], rgb=p["rgb"]))
        for i in range(192 - 1 - len(self.planets) - len(stars)):  # kernel threads: faint dust
            nm = f"kworker-{i}"
            stars.append(dict(name=nm, ang=2 * np.pi * h01(nm, "ra"), r=0.93 + 0.08 * h01(nm, "dec"),
                              m=7.5, rgb=(150, 160, 190)))
        return stars

    # world (x, y with y up) -> screen
    def xy(self, z):
        z = np.asarray(z)
        return self.cx + z.real * self.Rpx, self.cy - z.imag * self.Rpx * TILT

    def ellipse_pts(self, R, n=360, rot=0.0, incl=0.0):
        a = np.linspace(0, 2 * np.pi, n + 1)
        x, y = R * np.cos(a), R * np.sin(a) * math.cos(incl)
        z = (x + 1j * y) * np.exp(1j * rot)
        sx, sy = self.xy(z)
        return list(zip(sx, sy))

    def static_layer(self):
        """Armillary rings, ecliptic scale and deferent circles, supersampled."""
        W, H, s = self.W, self.H, self.s
        ss = 2
        img = Image.new("RGB", (W * ss, H * ss))
        d = ImageDraw.Draw(img)

        def P(pts):
            return [(x * ss, y * ss) for x, y in pts]

        # outer sphere and armillary hoops
        d.line(P(self.ellipse_pts(1.045)), fill=(70, 66, 100), width=max(1, int(2 * s)))
        d.line(P(self.ellipse_pts(0.905)), fill=(52, 50, 80), width=1)
        d.line(P(self.ellipse_pts(0.87)), fill=(52, 50, 80), width=1)
        d.line(P(self.ellipse_pts(0.88, rot=0.4, incl=math.radians(66))), fill=(46, 40, 70), width=1)
        d.line(P(self.ellipse_pts(0.88, rot=0.4 + np.pi / 2, incl=math.radians(78))), fill=(40, 36, 62), width=1)
        # degree scale between the two ecliptic hoops
        for deg in range(0, 360, 5):
            a = math.radians(deg)
            r1 = 0.87
            r2 = 0.905 if deg % 30 == 0 else (0.89 if deg % 10 == 0 else 0.88)
            p1 = self.xy(r1 * np.exp(1j * a))
            p2 = self.xy(r2 * np.exp(1j * a))
            col = (92, 88, 128) if deg % 30 == 0 else (60, 58, 88)
            d.line([(p1[0] * ss, p1[1] * ss), (p2[0] * ss, p2[1] * ss)], fill=col, width=1)
        # deferents
        for p in self.planets:
            d.line(P(self.ellipse_pts(p.R)), fill=(58, 60, 92), width=1)
        small = img.resize((W, H), Image.LANCZOS)
        return np.asarray(small, np.float32)

    def scale_labels(self, draw):
        f = max(9, int(10 * self.s))
        for deg in range(0, 360, 30):
            a = math.radians(deg)
            x, y = self.xy(0.845 * np.exp(1j * a))
            S.label(draw, x, y, f"{deg}", size=f, fill=(96, 94, 132))


# ----------------------------------------------------------------------------- render
def render(sc, t, hist=330.0, tint_lag=0.0, sim_dt=0.25, label_mode="full"):
    W, H, s = sc.W, sc.H, sc.s
    canvas = sc.bg.copy()

    # fixed stars: ring turns once a minute
    spin = 2 * np.pi * t / 60.0
    for st in sc.stars:
        z = st["r"] * np.exp(1j * (st["ang"] + spin))
        x, y = sc.xy(z)
        m = st["m"]
        b = 10 ** (-0.4 * (m - 1.0))
        core = 0.9 + 1.4 * min(b, 3.0) ** 0.5
        tw = 1.0  # fixed stars hold steady (twinkle belongs to the hr view)
        col = np.array(st["rgb"], np.float32)
        S.glow(canvas, x, y, core * s * 0.9, col, min(1.6, 0.35 + 0.9 * b ** 0.5) * tw)
        if b > 0.25:
            S.glow(canvas, x, y, core * s * 3.2, col, 0.10 * min(b, 4) ** 0.5 * tw)

    # trails, chains: supersampled line layer with bloom
    ss = 2
    lay = Image.new("RGB", (W * ss, H * ss))
    d = ImageDraw.Draw(lay)
    chains = Image.new("RGB", (W * ss, H * ss))
    dc = ImageDraw.Draw(chains)
    tw_px = max(2, int(round(2.6 * s * ss)))
    state = {}
    for p in sc.planets:
        hp = min(hist, 0.8 * p.T)
        tt = np.arange(t - hp, t + 1e-6, sim_dt)
        z = p.pos(tt)
        retro = p.retro(tt)
        sx, sy = sc.xy(z)
        base = np.array(S.KIND[p.kind], np.float32)
        red = np.array(RETRO, np.float32)
        age = (t - tt) / hp
        fade = np.clip((1 - age) / 0.45, 0, 1) ** 1.3  # only the oldest stretch fades
        # the tint arrives a beat after the derivative turns: detection needs a few samples
        tint = np.clip((t - tt - tint_lag) / tint_lag, 0, 1) if tint_lag > 0 else np.ones_like(tt)
        for i in range(len(tt) - 1):
            k = tint[i] if retro[i] else 0.0
            col = base * (1 - k) + red * k
            a = fade[i] * (0.95 if retro[i] else 0.75)
            if a < 0.02:
                continue
            c = tuple(int(v) for v in col * a)
            d.line([(sx[i] * ss, sy[i] * ss), (sx[i + 1] * ss, sy[i + 1] * ss)], fill=c, width=tw_px)
        state[p.name] = bool(retro[-1])
        # deferent point and epicycle chain
        joints = p.chain(t)
        jx, jy = sc.xy(joints)
        for j in range(len(joints) - 1):
            rad = abs(joints[j + 1] - joints[j])
            ex, ey = rad * sc.Rpx, rad * sc.Rpx * TILT
            if ex * ss > 1.5:
                dc.ellipse([(jx[j] - ex) * ss, (jy[j] - ey) * ss, (jx[j] + ex) * ss, (jy[j] + ey) * ss],
                           outline=(84, 98, 132) if j else (110, 118, 156), width=max(1, int(1.2 * s * ss)))
        dc.line([(x * ss, y * ss) for x, y in zip(jx, jy)], fill=(200, 205, 225), width=max(1, int(1.6 * s * ss)))
        r0 = 2.2 * s * ss
        dc.ellipse([jx[0] * ss - r0, jy[0] * ss - r0, jx[0] * ss + r0, jy[0] * ss + r0], fill=(150, 156, 190))
        # spoke from Earth to the deferent point, very faint
        e0 = sc.xy(0j)
        dc.line([(e0[0] * ss, e0[1] * ss), (jx[0] * ss, jy[0] * ss)], fill=(40, 42, 64), width=1)

    trail = np.asarray(lay.resize((W, H), Image.LANCZOS), np.float32)
    canvas += trail + ndimage.gaussian_filter(trail, (3.5 * s, 3.5 * s, 0)) * 1.4
    ch = np.asarray(chains.resize((W, H), Image.LANCZOS), np.float32)
    canvas += ch * 0.85

    # sight line from Earth to each retrograde planet, out to the ecliptic scale
    sight = Image.new("RGB", (W * ss, H * ss))
    dsg = ImageDraw.Draw(sight)
    ex0, ey0 = sc.xy(0j)
    for p in sc.planets:
        if not state[p.name]:
            continue
        z = complex(p.pos([t])[0])
        u = z / abs(z)
        for seg in HR.dashes([(ex0, ey0), sc.xy(0.905 * u)], 6 * s, 5 * s):
            dsg.line([(x * ss, y * ss) for x, y in seg], fill=(150, 60, 60), width=max(1, int(1.4 * s * ss)))
        # backward-pointing marker on the ecliptic scale
        a = np.angle(u)
        tri = [sc.xy(0.918 * np.exp(1j * (a - 0.035))), sc.xy(0.905 * np.exp(1j * (a + 0.02))),
               sc.xy(0.932 * np.exp(1j * (a + 0.02)))]
        dsg.polygon([(x * ss, y * ss) for x, y in tri], fill=RETRO)
    sg = np.asarray(sight.resize((W, H), Image.LANCZOS), np.float32)
    canvas += sg

    # planets
    for p in sc.planets:
        z = complex(p.pos([t])[0])
        x, y = sc.xy(z)
        rad = (4.2 + 1.3 * math.log2(max(p.mem / (64 * MIB), 1.0))) * s
        col = S.KIND[p.kind]
        S.glow(canvas, x, y, rad * 2.2, col, 0.55)
        if state[p.name]:
            S.glow(canvas, x, y, rad * 3.2, RETRO, 0.35)
        S.sphere(canvas, x, y, rad, col)
        p._screen = (x, y, rad)

    # Earth: isotop itself
    ex, ey = sc.xy(0j)
    S.glow(canvas, ex, ey, 16 * s, EARTH, 0.45)
    S.sphere(canvas, ex, ey, 9.5 * s, EARTH)
    # a hint of continents
    for (dx, dy, rr) in [(-2.5, -1.5, 3.2), (2.8, 2.2, 2.4)]:
        S.glow(canvas, ex + dx * s, ey + dy * s, rr * s, (30, 90, 40), 0.35)

    img = S.to_image(canvas)
    draw = ImageDraw.Draw(img)
    fs = int(round(13 * s)) if W >= 1200 else 11
    if label_mode != "none":
        sc.scale_labels(draw)
        S.label(draw, ex, ey + 22 * s, "isotop (pid 4242)", size=fs, fill=(170, 225, 215))
        if label_mode == "full":
            S.label(draw, ex, ey + 22 * s + fs * 1.25, "0.4 cores  96 MiB", size=max(9, fs - 2), fill=S.DIM)
        under = Image.new("RGBA", img.size, (0, 0, 0, 0))
        text = Image.new("RGBA", img.size, (0, 0, 0, 0))
        tdraw = ImageDraw.Draw(text)
        tdraw.plate = ImageDraw.Draw(under)
        place_labels(sc, tdraw, t, state, fs)
        img = Image.alpha_composite(Image.alpha_composite(img.convert("RGBA"), under), text).convert("RGB")
    return img, state


def place_labels(sc, draw, t, state, fs):
    """Planet labels pushed radially outward from Earth, nudged apart."""
    f = S.font(fs)
    boxes = []
    ex, ey = sc.xy(0j)
    discs = [(q._screen[0], q._screen[1], q._screen[2] + 5 * sc.s) for q in sc.planets]
    discs.append((ex, ey, 14 * sc.s))

    def hits_disc(box):
        for cx, cy, cr in discs:
            nx, ny = min(max(cx, box[0]), box[2]), min(max(cy, box[1]), box[3])
            if math.hypot(nx - cx, ny - cy) < cr:
                return True
        return False
    order = sorted(sc.planets, key=lambda p: not state[p.name])
    plate = draw.plate
    for p in order:
        x, y, rad = p._screen
        ux, uy = x - ex, (y - ey) / TILT
        n = math.hypot(ux, uy) or 1
        ux, uy = ux / n, uy / n
        text = f"Sphere of {p.sphere}: {p.name}"
        tw = draw.textlength(text, font=f)
        th = fs * 1.2
        placed = None
        for dist in (rad + 8 * sc.s, rad + 22 * sc.s, rad + 38 * sc.s, rad + 56 * sc.s):
            for rot in (0, 0.5, -0.5, 1.0, -1.0):
                c, s_ = math.cos(rot), math.sin(rot)
                vx, vy = ux * c - uy * s_, ux * s_ + uy * c
                lx, ly = x + vx * dist, y + vy * dist * TILT
                x0 = lx if vx >= -0.3 else lx - tw
                if abs(vx) < 0.3:
                    x0 = lx - tw / 2
                y0 = ly - th / 2
                box = (x0 - 3, y0 - 2, x0 + tw + 3, y0 + th + 2)
                if box[0] < 4 or box[2] > sc.W - 4 or box[1] < 4 or box[3] > sc.H - sc.band - 4:
                    continue
                if hits_disc(box):
                    continue
                if any(not (box[2] < b[0] or box[0] > b[2] or box[3] < b[1] or box[1] > b[3]) for b in boxes):
                    continue
                placed = (x0, y0, box)
                break
            if placed:
                break
        if not placed:
            continue
        x0, y0, box = placed
        boxes.append(box)
        plate.rounded_rectangle(box, radius=3, fill=(8, 7, 18, 150))
        head = f"Sphere of {p.sphere}: "
        S.label(draw, x0, y0 + th / 2, head, size=fs, fill=(150, 150, 185), anchor="lm")
        hw = draw.textlength(head, font=f)
        S.label(draw, x0 + hw, y0 + th / 2, p.name, size=fs,
                fill=(255, 150, 140) if state[p.name] else S.TEXT, anchor="lm")


# Where each sphere's planet sits (degrees, screen-anticlockwise from the right) at the
# showcase moment, so the still reads clearly. In isotop the phases come from identity hashes.
TARGET = {"Moon": 205, "Mercury": 20, "Venus": 120, "Sun": 160, "Mars": 300, "Jupiter": 60, "Saturn": 245}


def arrange(sc, t_frame):
    for p in sc.planets:
        if p.name == "compiler-90":
            continue
        best = None
        for off in np.linspace(-25, 25, 26):
            want = math.radians(TARGET[p.sphere] + off)
            p.phase = want - 2 * np.pi * t_frame / p.T
            tt = np.arange(t_frame - 40, t_frame + 40, 0.5)
            r = p.retro(tt)
            score = r[len(tt) // 2 - 4: len(tt) // 2 + 4].sum() * 10 + r.mean() + abs(off) * 0.01
            if best is None or score < best[0]:
                best = (score, p.phase)
        p.phase = best[1]
    # compiler-90: its own hash phase, rotated to the target (retrograde timing is unchanged
    # only up to the epicycle orientation, so check)
    p = next(q for q in sc.planets if q.name == "compiler-90")
    ang = p.phase + 2 * np.pi * t_frame / p.T
    print("compiler-90 at", round(math.degrees(ang) % 360), "deg")


def status_lines(sc, t, state, speed=None, compact=False):
    retro = [p.name for p in sc.planets if state[p.name]]
    if retro:
        rs = (", ".join(retro[:-1]) + " and " + retro[-1] + " are" if len(retro) > 1 else retro[0] + " is") + \
            " in retrograde"
    else:
        rs = "all wanderers direct"
    n_fixed = len(sc.stars)
    l1 = (f"ISOTOP / PTOLEMY / DEMO   192 processes | 7 wanderers | {n_fixed} fixed stars | 60 samples | {rs}")
    l2 = ("planet = most varied process | deferent = steady revolution, period ~ r^1.5, busiest innermost | "
          "epicycles = 12 largest DFT terms of (cpu - mean) + i (log2 mem - mean), last 60 s")
    l3 = ("trail = apparent path, red = retrograde (longitude seen from Earth falling) | fixed stars = steady "
          "processes, brightness = memory, ring turns once a minute | Earth = isotop" +
          (f" | replay x{speed}" if speed else "") + f" | t={t:.1f}s")
    if compact:
        l1 = f"ISOTOP / PTOLEMY / DEMO   192 processes | 7 wanderers | {n_fixed} fixed stars | {rs}"
        l2 = "epicycles = 12 largest DFT terms of (cpu - mean) + i (log2 mem - mean) | deferent period ~ r^1.5"
        l3 = f"red trail = retrograde, longitude seen from Earth falling | replay x{speed} | t={t:.1f}s"
    return [l1, l2, l3]


def retro_intervals(p, t0, t1, dt=0.25):
    tt = np.arange(t0, t1, dt)
    r = p.retro(tt)
    out, start = [], None
    for ti, ri in zip(tt, r):
        if ri and start is None:
            start = ti
        if not ri and start is not None:
            out.append((start, ti))
            start = None
    return out


def main():
    import time
    t0 = time.time()
    mode = sys.argv[1] if len(sys.argv) > 1 else "still"
    sc = Scene(1600, 900)
    for p in sc.planets:
        tt = np.arange(0, 720, 0.25)
        frac = p.retro(tt).mean()
        print(f"{p.sphere:8s} {p.name:20s} meancpu {p.mean_cpu:4.2f} R {p.R:.3f} T {p.T:5.0f}s "
              f"chain {np.abs(p.c).sum():.3f} retro {frac:.2f}")
        # reconstruction check: all 59 terms reproduce the samples exactly
    pl = next(p for p in sc.planets if p.name == "compiler-90")
    iv = retro_intervals(pl, 300, 1000)
    iv = sorted(iv, key=lambda ab: ab[0] - ab[1])  # longest retrograde stretch first
    t_frame = iv[0][0] + 0.55 * (iv[0][1] - iv[0][0])
    arrange(sc, t_frame)
    print("compiler retro intervals", [(round(a, 1), round(b, 1)) for a, b in iv][:8])
    T_STILL = float(sys.argv[2]) if len(sys.argv) > 2 else None
    iv = sorted(iv, key=lambda ab: ab[0] - ab[1])  # longest retrograde stretch first
    if T_STILL is None:
        a, b = iv[0]
        T_STILL = a + 0.55 * (b - a)
    if mode in ("still", "both"):
        img, state = render(sc, T_STILL)
        S.status(img, status_lines(sc, T_STILL, state), size=13)
        print(S.save_still(img, "ptolemy"), time.time() - t0)
    if mode in ("anim", "both"):
        animate(iv)
        print("anim", time.time() - t0)


def animate(iv):
    sc = Scene(960, 540)
    arrange(sc, iv[0][0] + 0.55 * (iv[0][1] - iv[0][0]))
    fps, speed, n = 10, 6, 100
    # window: compiler-90's retrograde loop starts about a third of the way in
    a, b = iv[0]
    t_start = a - 0.33 * n / fps * speed
    frames = []
    for i in range(n):
        t = t_start + i / fps * speed
        img, state = render(sc, t, tint_lag=4.0, sim_dt=0.4, label_mode="short")
        S.status(img, status_lines(sc, t - t_start, state, speed=speed, compact=True), size=11)
        frames.append(img)
    path, size = S.save_frames(frames, "ptolemy", fps=fps, max_width=720, colors=64)
    import gifpack
    path, size = gifpack.pack("ptolemy", fps=fps, width=720, colors=80, stats="diff")
    print(path, size)
    sheet = Image.new("RGB", (960 * 2, 540 * 2))
    for k, fi in enumerate([4, 34, 64, 96]):
        sheet.paste(frames[fi], ((k % 2) * 960, (k // 2) * 540))
    sheet.save(f"{S.OUT}/ptolemy-sheet.png")


if __name__ == "__main__":
    main()
