"""isotop `hr` view mockup: a Hertzsprung-Russell sky.

Real computation here (what the view would do with real samples):
- Blackbody colours: Planck's law integrated against CIE 1931 2-degree colour matching
  functions (Wyman, Sloan and Shirley 2013 multi-lobe fit), XYZ -> linear sRGB, normalised to
  the brightest channel, sRGB-encoded. Table 1,000 K to 40,000 K (building block B5).
- T = 2400 K x (1 + cpu/5)^0.47, magnitude m = -2.5 log10(mem / 1 GiB), spectral class and
  subclass from T, Theil-Sen main sequence fit of m on log T, luminosity class from the residual.
- Twinkle amplitude = coefficient of variation of each process's 10 s CPU history.
- Constellations = cgroups on a coarse RA/Dec cell grid (stepped IAU-style boundaries), stars
  placed by identity hash with relaxation, stick figures along (synthesised) socket links.
Procedural mockup: the demo population itself, the socket links, Milky Way texture (its
brightness is the kernel thread count), dome rim decoration.
"""
import hashlib
import math
import sys

sys.path.insert(0, "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/proto")
import isostyle as S  # noqa: E402

import numpy as np  # noqa: E402
from PIL import Image, ImageDraw  # noqa: E402
from scipy import ndimage  # noqa: E402

GIB = 1024 ** 3
MIB = 1024 ** 2


# ----------------------------------------------------------------------------- utilities
def h01(name, salt=""):
    d = hashlib.blake2b((salt + name).encode(), digest_size=8).digest()
    return int.from_bytes(d, "little") / 2 ** 64


def rng_for(name):
    return np.random.default_rng(int(h01(name, "rng") * 2 ** 32))


def smoothstep(a, b, x):
    t = min(max((x - a) / (b - a), 0.0), 1.0)
    return t * t * (3 - 2 * t)


def ease(x):
    x = min(max(x, 0.0), 1.0)
    return 4 * x ** 3 if x < 0.5 else 1 - (-2 * x + 2) ** 3 / 2


class Ink:
    """Supersampled greyscale ink layer for antialiased line art."""

    def __init__(self, w, h, ss=3):
        self.ss, self.w, self.h = ss, w, h
        self.img = Image.new("L", (w * ss, h * ss), 0)
        self.d = ImageDraw.Draw(self.img)

    def line(self, pts, width, value=255):
        s = self.ss
        if len(pts) < 2:
            return
        self.d.line([(x * s, y * s) for x, y in pts], fill=int(value), width=max(1, round(width * s)),
                    joint="curve")

    def ellipse(self, cx, cy, rx, ry, width, value=255):
        s = self.ss
        self.d.ellipse([(cx - rx) * s, (cy - ry) * s, (cx + rx) * s, (cy + ry) * s], outline=int(value),
                       width=max(1, round(width * s)))

    def mask(self):
        return np.asarray(self.img.reduce(self.ss), np.float32) / 255.0


def over(canvas, mask, colour, alpha=1.0):
    a = (mask * alpha)[..., None]
    canvas[:] = canvas * (1 - a) + np.array(colour, np.float32) * a


def add(canvas, mask, colour, alpha=1.0):
    canvas += (mask * alpha)[..., None] * np.array(colour, np.float32)


def dashes(pts, on, off, phase=0.0):
    """Split a polyline into dash polylines by arc length."""
    out, cur = [], []
    period = on + off
    s = phase
    for (x0, y0), (x1, y1) in zip(pts[:-1], pts[1:]):
        seg = math.hypot(x1 - x0, y1 - y0)
        if seg == 0:
            continue
        pos = 0.0
        while pos < seg:
            inside = (s % period) < on
            step = (on - s % period) if inside else (period - s % period)
            step = min(max(step, 1e-6 * period), seg - pos)
            a = pos / seg
            b = (pos + step) / seg
            pa = (x0 + (x1 - x0) * a, y0 + (y1 - y0) * a)
            pb = (x0 + (x1 - x0) * b, y0 + (y1 - y0) * b)
            if inside:
                if not cur:
                    cur = [pa]
                cur.append(pb)
            elif cur:
                out.append(cur)
                cur = []
            pos += step
            s += step
    if cur:
        out.append(cur)
    return out


def smallcaps(draw, x, y, text, size, fill, tracking=0.25, path=S.FONT_MONO, shadow=True, anchor="m"):
    """Small caps: capital-height initials, 0.74 size for the rest, letter-spaced."""
    big = S.font(size, path)
    small = S.font(int(round(size * 0.74)), path)
    glyphs = []
    for i, ch in enumerate(text):
        initial = i == 0
        f = big if (initial and ch.isalpha()) or ch.isupper() else small
        glyphs.append((ch.upper(), f))
    widths = [f.getlength(c) + tracking * size for c, f in glyphs]
    total = sum(widths) - tracking * size
    cx = x - total / 2 if anchor == "m" else x
    for (c, f), wdt in zip(glyphs, widths):
        if shadow:
            draw.text((cx + 1, y + 1), c, font=f, fill=(0, 0, 0), anchor="ls")
        draw.text((cx, y), c, font=f, fill=fill, anchor="ls")
        cx += wdt
    return total


# ----------------------------------------------------------------------------- blackbody (B5)
def _g(lam, mu, s1, s2):
    s = np.where(lam < mu, s1, s2)
    return np.exp(-0.5 * ((lam - mu) / s) ** 2)


def cie1931(lam):
    x = 1.056 * _g(lam, 599.8, 37.9, 31.0) + 0.362 * _g(lam, 442.0, 16.0, 26.7) - 0.065 * _g(lam, 501.1, 20.4, 26.2)
    y = 0.821 * _g(lam, 568.8, 46.9, 40.5) + 0.286 * _g(lam, 530.9, 16.3, 31.1)
    z = 1.217 * _g(lam, 437.0, 11.8, 36.0) + 0.681 * _g(lam, 459.0, 26.0, 13.8)
    return x, y, z


def _srgb_encode(c):
    return np.where(c <= 0.0031308, 12.92 * c, 1.055 * np.power(np.clip(c, 0, None), 1 / 2.4) - 0.055)


def blackbody_table():
    lam = np.arange(360.0, 831.0, 1.0)
    xb, yb, zb = cie1931(lam)
    temps = np.arange(1000.0, 40001.0, 50.0)
    lm = lam * 1e-9
    c2 = 1.4387769e-2
    planck = 1.0 / (lm[None, :] ** 5 * (np.exp(c2 / (lm[None, :] * temps[:, None])) - 1.0))
    X = planck @ xb
    Y = planck @ yb
    Z = planck @ zb
    M = np.array([[3.2406, -1.5372, -0.4986], [-0.9689, 1.8758, 0.0415], [0.0557, -0.2040, 1.0570]])
    rgb = np.stack([X, Y, Z], 1) @ M.T
    rgb = np.clip(rgb, 0, None)
    rgb /= rgb.max(1, keepdims=True)
    return temps, np.clip(_srgb_encode(rgb), 0, 1) * 255


BB_T, BB_RGB = blackbody_table()


def bb_colour(T):
    T = min(max(T, 1000.0), 40000.0)
    return tuple(float(np.interp(T, BB_T, BB_RGB[:, i])) for i in range(3))


# ----------------------------------------------------------------------------- mapping
def temperature(cpu):
    return min(2400.0 * (1.0 + cpu / 5.0) ** 0.47, 40000.0)


def magnitude(mem):
    return -2.5 * math.log10(mem / GIB)


CLASSES = [("O", 30000, 50000), ("B", 10000, 30000), ("A", 7500, 10000), ("F", 6000, 7500),
           ("G", 5200, 6000), ("K", 3700, 5200), ("M", 2400, 3700)]


def spectral(T):
    for letter, lo, hi in CLASSES:
        if T >= lo or letter == "M":
            f = (math.log10(hi) - math.log10(T)) / (math.log10(hi) - math.log10(lo))
            return letter, int(min(max(math.floor(10 * f), 0), 9))


# luminosity classes from the residual r = m - m_fit (magnitudes, + is fainter). Convention.
LUM = [(-6.0, "Ia"), (-4.5, "Ib"), (-3.25, "II"), (-1.75, "III"), (-0.9, "IV"), (1.5, "V"), (2.75, "VI")]


def lum_class(resid, T):
    if resid >= 2.75:
        return "D" if T >= 7500 else "VI"
    for edge, name in LUM:
        if resid < edge:
            return name
    return "VI"


def theil_sen(x, y, min_dx=0.01):
    i, j = np.triu_indices(len(x), 1)
    dx = x[j] - x[i]
    ok = np.abs(dx) > min_dx
    slope = np.median((y[j] - y[i])[ok] / dx[ok])
    icpt = np.median(y - slope * x)
    return icpt, slope


# ----------------------------------------------------------------------------- demo population
def build_population():
    pop = []

    def add(name, group, cpu, mem_mib, kind="system", jitter=None):
        r = rng_for(name)
        if jitter is None:
            jitter = 0.12 if cpu > 30 else (0.45 if cpu > 3 else 1.3)
        hist = np.clip(cpu * (1.0 + jitter * r.standard_normal(10)), 0, None)
        if cpu < 2:
            # idle daemons: mostly asleep with the odd wakeup
            hist = np.where(r.random(10) < 0.3, r.exponential(cpu * 3 + 0.2, 10), cpu * 0.2)
        pop.append(dict(name=name, group=group, cpu=float(cpu), mem=float(mem_mib) * MIB, kind=kind,
                        hist=hist))

    def filler(name, group, kind, cpu_scale, base_mib=9.0, sigma=0.35, cpu=None, giant=False):
        r = rng_for(name)
        c = cpu if cpu is not None else float(r.exponential(cpu_scale))
        if giant:  # big for its CPU: browser renderers sit on a giant branch
            mem = base_mib * (1 + c / 5.0) ** 0.35 * math.exp(sigma * r.standard_normal())
        else:  # main sequence: memory grows with work done
            mem = base_mib * (1 + c / 5.0) ** 1.17 * math.exp(sigma * r.standard_normal())
        add(name, group, round(c, 2), mem, kind)

    # system.slice: many small idle daemons, a few medium ones
    for n, mib in [("journald-310", 38), ("udevd-322", 9), ("dbus-112", 6), ("NetworkManager-640", 22),
                   ("polkitd-655", 11), ("accounts-daemon-650", 8), ("avahi-daemon-612", 4),
                   ("avahi-daemon-618", 2), ("cupsd-702", 12), ("bluetoothd-611", 7),
                   ("ModemManager-688", 14), ("chronyd-603", 3), ("cron-605", 3), ("sshd-760", 9),
                   ("rsyslogd-614", 6), ("thermald-622", 10), ("upowerd-1102", 9), ("udisksd-648", 15),
                   ("snapd-660", 42), ("containerd-781", 55), ("dockerd-905", 88), ("gdm-880", 13),
                   ("colord-1240", 16), ("fwupd-1630", 96), ("packagekitd-1410", 64),
                   ("systemd-logind-620", 8)]:
        r = rng_for(n)
        add(n, "system.slice", round(float(r.exponential(0.6)), 2), mib)
    add("systemd-1", "init.scope", 0.3, 14)

    # postgresql.service
    add("postgres-41", "postgresql.service", 15.0, 600)
    add("postgres-44", "postgresql.service", 1.0, 210)  # checkpointer: big, idle
    add("postgres-45", "postgresql.service", 2.0, 30)
    add("postgres-46", "postgresql.service", 3.0, 45)
    add("postgres-47", "postgresql.service", 1.0, 40)
    add("postgres-48", "postgresql.service", 0.5, 20)
    for i in range(7):
        filler(f"postgres-{420 + i}", "postgresql.service", "system", 14)
    # nginx.service
    add("nginx-128", "nginx.service", 0.1, 8)
    for i in range(6):
        filler(f"nginx-{129 + i}", "nginx.service", "system", 6)
    # app-browser.scope
    add("browser-512", "app-browser.scope", 5.0, 1650, "session")
    add("browser-530", "app-browser.scope", 25.0, 420, "session")
    add("browser-533", "app-browser.scope", 3.0, 90, "session")
    for i in range(17):
        filler(f"browser-{600 + 7 * i}", "app-browser.scope", "session", 7, 110, 0.5, giant=True)
    # build.scope
    add("make-88", "build.scope", 1.0, 6, "session")
    add("ninja-89", "build.scope", 2.0, 10, "session")
    add("compiler-90", "build.scope", 97.0, 520, "session")
    for i, (c, mib) in enumerate([(98, 340), (92, 410), (85, 260), (99, 300), (64, 180), (78, 230)]):
        add(f"cc1plus-{91 + i}", "build.scope", c, mib, "session")
    add("ld-101", "build.scope", 58.0, 330, "session")
    # session-3.scope
    for n, c, mib in [("shell-170", 0.0, 5), ("tmux-160", 0.5, 8), ("zsh-171", 0.0, 5), ("zsh-172", 0.1, 6),
                      ("zsh-173", 0.0, 4), ("vim-301", 1.0, 30), ("ffmpeg-999", 86.0, 12),
                      ("htop-305", 3.0, 6), ("ssh-agent-158", 0.0, 2), ("less-310", 0.0, 3),
                      ("python3-320", 12.0, 120)]:
        add(n, "session-3.scope", c, mib, "session")
    # user@1000.service
    for n, c, mib in [("pipewire-95", 4.0, 25), ("wireplumber-97", 1.0, 30), ("pipewire-pulse-98", 1.0, 20),
                      ("gnome-shell-1500", 6.0, 650), ("xdg-portal-1510", 0.2, 25), ("gvfsd-1520", 0.0, 8),
                      ("gvfsd-1521", 0.0, 7), ("gvfsd-1522", 0.1, 9), ("evolution-1530", 0.3, 40),
                      ("tracker-miner-1540", 0.3, 120), ("keyring-1550", 0.0, 10), ("dbus-1560", 0.5, 6),
                      ("xwayland-1570", 3.0, 120), ("gsd-power-1580", 0.2, 18), ("gsd-color-1581", 0.1, 22),
                      ("gsd-media-1582", 0.1, 16), ("gsd-xsettings-1583", 0.3, 30), ("gsd-wacom-1584", 0.0, 15),
                      ("ibus-1600", 0.4, 20), ("ibus-engine-1601", 0.2, 12)]:
        add(n, "user@1000.service", c, mib, "session")
    # containers
    add("java-elastic-77", "docker-4f1a.scope", 6.0, 6600, "container")
    add("tini-75", "docker-4f1a.scope", 0.0, 1, "container")
    add("logstash-79", "docker-4f1a.scope", 4.0, 1150, "container")
    add("redis-207", "docker-9c2e.scope", 8.0, 180, "container")
    add("worker-134", "docker-9c2e.scope", 250.0, 900, "container")
    for i in range(5):
        filler(f"worker-{135 + i}", "docker-9c2e.scope", "container", 25)
    add("gunicorn-140", "docker-9c2e.scope", 1.0, 60, "container")
    add("celery-beat-141", "docker-9c2e.scope", 0.5, 50, "container")
    add("tini-133", "docker-9c2e.scope", 0.0, 1, "container")
    # app-code.scope
    add("code-311", "app-code.scope", 7.0, 500, "session")
    add("language-server-71", "app-code.scope", 30.0, 700, "session")
    add("code-312", "app-code.scope", 12.0, 300, "session")
    add("code-313", "app-code.scope", 2.0, 80, "session")
    add("node-314", "app-code.scope", 4.0, 120, "session")
    # ollama.service: one very hot, very big star
    add("llama-server-404", "ollama.service", 1500.0, 9600, "system", jitter=0.06)
    add("ollama-400", "ollama.service", 0.5, 60)
    # backup.service (batch-import-301 goes supernova in the animation)
    add("batch-import-301", "backup.service", 22.0, 2150)
    add("rclone-302", "backup.service", 15.0, 80)
    add("gzip-303", "backup.service", 95.0, 3, jitter=0.05)
    add("restic-304", "backup.service", 40.0, 300)
    add("sh-300", "backup.service", 0.0, 2)

    for p in pop:
        p["T"] = temperature(p["cpu"])
        p["logT"] = math.log10(p["T"])
        p["m"] = magnitude(p["mem"])
        h = p["hist"]
        p["cv"] = float(np.std(h) / max(np.mean(h), 1e-3)) if np.mean(h) > 1e-3 else 0.0
        p["twinkle"] = min(0.8, 0.45 * p["cv"])
        p["rgb"] = bb_colour(p["T"])
    x = np.array([p["logT"] for p in pop])
    y = np.array([p["m"] for p in pop])
    a, b = theil_sen(x, y)
    for p in pop:
        p["resid"] = p["m"] - (a + b * p["logT"])
        letter, sub = spectral(p["T"])
        lc = lum_class(p["resid"], p["T"])
        p["cls"] = f"D{letter}" if lc == "D" else f"{letter}{sub} {lc}"
    kernel = [f"kworker-{i}" for i in range(59)]
    return pop, kernel, (a, b)


GROUPS = ["system.slice", "init.scope", "postgresql.service", "nginx.service", "app-browser.scope",
          "build.scope", "session-3.scope", "user@1000.service", "docker-4f1a.scope", "docker-9c2e.scope",
          "app-code.scope", "ollama.service", "backup.service"]

# Seeds for constellation regions, as fractions of the sky area on screen. In isotop these come
# from a hash of the cgroup path with relaxation; fixed here so the mockup frames are stable.
SEEDS = {
    "system.slice": (0.13, 0.30), "user@1000.service": (0.11, 0.78), "app-browser.scope": (0.33, 0.66),
    "app-code.scope": (0.37, 0.30), "init.scope": (0.24, 0.06), "postgresql.service": (0.58, 0.26),
    "nginx.service": (0.80, 0.10), "ollama.service": (0.92, 0.36), "docker-4f1a.scope": (0.74, 0.47),
    "build.scope": (0.57, 0.66), "session-3.scope": (0.40, 0.95), "docker-9c2e.scope": (0.88, 0.76),
    "backup.service": (0.70, 0.94),
}

# The labelled stars get hand-picked spots (in isotop: identity hash plus relaxation).
PINS = {"compiler-90": (0.585, 0.60), "postgres-41": (0.655, 0.335), "java-elastic-77": (0.775, 0.43),
        "llama-server-404": (0.905, 0.285), "ffmpeg-999": (0.50, 0.835), "batch-import-301": (0.80, 0.80)}

DL, DB = 7.5, 5.0  # RA/Dec cell size of the boundary grid (degrees)
L0, L1, B0, B1 = -127.5, 127.5, -60.0, 85.0


def sky_layout(pop, cam, W, Hs):
    nl = int(round((L1 - L0) / DL))
    nb = int(round((B1 - B0) / DB))
    lc = L0 + (np.arange(nl) + 0.5) * DL
    bc = B0 + (np.arange(nb) + 0.5) * DB
    LL, BB = np.meshgrid(lc, bc, indexing="ij")
    cx, cy = cam.project(LL, BB)
    # screen area of each cell from its corners (shoelace)
    xs, ys = [], []
    for dl, db in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)]:
        x, y = cam.project(LL + dl * DL, BB + db * DB)
        xs.append(x)
        ys.append(y)
    area = 0.5 * np.abs(sum(xs[k] * ys[(k + 1) % 4] - xs[(k + 1) % 4] * ys[k] for k in range(4)))
    vis = (cx > -60) & (cx < W + 60) & (cy > -60) & (cy < Hs + 50) & (BB < 84)
    inner = (cx > 45) & (cx < W - 45) & (cy > 34) & (cy < Hs - 40)
    counts = {g: sum(1 for p in pop if p["group"] == g) for g in GROUPS}
    want = np.array([(counts[g] + 3) ** 0.8 for g in GROUPS])
    total = (area * vis).sum()
    want = want / want.sum() * total
    r = np.random.default_rng(7)
    noise = ndimage.gaussian_filter(r.standard_normal((nl, nb)), 1.6)
    noise /= np.abs(noise).max()
    seeds = {g: (u * W, v * Hs) for g, (u, v) in SEEDS.items()}
    dist = np.stack([np.hypot(cx - seeds[g][0], cy - seeds[g][1]) for g in GROUPS], 0)
    w = np.zeros(len(GROUPS))
    for _ in range(300):
        score = dist ** 2 - w[:, None, None] + 9000 * noise[None]
        owner = np.argmin(score, 0)
        owner[~vis] = -1
        got = np.array([(area * (owner == k)).sum() for k in range(len(GROUPS))])
        w += 0.5 * (want - got) / want.mean() * 8000
    pos = {}
    for gi, g in enumerate(GROUPS):
        members = [p for p in pop if p["group"] == g]
        ok = (owner == gi) & inner
        cells = np.argwhere(ok)
        cw = area[ok]
        if len(cells) == 0:
            raise SystemExit(f"region {g} has no visible cells")
        pts = []
        for p in members:
            rr = rng_for(p["name"])
            k = rr.choice(len(cells), p=cw / cw.sum())
            i, j = cells[k]
            pts.append([lc[i] + (rr.random() - 0.5) * DL * 0.9, bc[j] + (rr.random() - 0.5) * DB * 0.9])
        pts = np.array(pts, float)
        fixed = np.zeros(len(pts), bool)
        for a, p in enumerate(members):
            if p["name"] in PINS:
                u, v = PINS[p["name"]]
                vv = cam.unproject(np.array([u * W]), np.array([v * Hs]))[0]
                lam, bet = math.degrees(math.atan2(vv[1], vv[0])), math.degrees(math.asin(vv[2]))
                ci, cj = int((lam - L0) // DL), int((bet - B0) // DB)
                if owner[ci, cj] != gi:
                    print("pin outside its region:", p["name"], GROUPS[owner[ci, cj]])
                pts[a] = (lam, bet)
                fixed[a] = True

        def scr(q):
            x, y = cam.project(np.array([q[0]]), np.array([q[1]]))
            return float(x[0]), float(y[0])

        for _ in range(50):
            sp = np.array([scr(q) for q in pts])
            for a in range(len(pts)):
                if fixed[a]:
                    continue
                d = sp[a] - sp
                dd = np.hypot(d[:, 0], d[:, 1]) + 1e-6
                push = np.clip(70.0 - dd, 0, None)
                push[a] = 0
                f = (d / dd[:, None] * push[:, None]).sum(0) * 0.12
                if not f.any():
                    continue
                # convert a screen step into a (lambda, beta) step with a finite difference Jacobian
                x0, y0 = sp[a]
                xl, yl = scr(pts[a] + [0.5, 0])
                xb, yb = scr(pts[a] + [0, 0.5])
                J = np.array([[xl - x0, xb - x0], [yl - y0, yb - y0]]) / 0.5
                step = np.linalg.lstsq(J, f, rcond=None)[0]
                cand = pts[a] + np.clip(step, -3, 3)
                ci = int((cand[0] - L0) // DL)
                cj = int((cand[1] - B0) // DB)
                if 0 <= ci < nl and 0 <= cj < nb and owner[ci, cj] == gi and inner[ci, cj]:
                    pts[a] = cand
                    sp[a] = scr(cand)
        for p, q in zip(members, pts):
            pos[p["name"]] = (float(q[0]), float(q[1]))
    return dict(owner=owner, lc=lc, bc=bc, nl=nl, nb=nb, pos=pos)


def socket_links(pop, pos):
    """Demo socket links: a spanning stick figure over each cgroup's brighter stars plus a few
    cross-cgroup client links. (In isotop these are Snapshot.links.)"""
    links = []
    for g in GROUPS:
        mem = sorted([p for p in pop if p["group"] == g], key=lambda p: p["m"])
        mem = mem[:max(2, min(9, int(len(mem) * 0.6)))]
        if len(mem) < 2:
            continue
        pts = np.array([pos[p["name"]] for p in mem])
        inside = [0]
        while len(inside) < len(mem):
            best = None
            for a in inside:
                for b in range(len(mem)):
                    if b in inside:
                        continue
                    d = pts[a] - pts[b]
                    d[0] *= math.cos(math.radians(pts[a][1]))
                    dd = math.hypot(*d)
                    if best is None or dd < best[0]:
                        best = (dd, a, b)
            inside.append(best[2])
            links.append((mem[best[1]]["name"], mem[best[2]]["name"], True))
    for a, b in [("worker-134", "postgres-422"), ("worker-136", "redis-207"), ("nginx-130", "gunicorn-140"),
                 ("browser-530", "nginx-131"), ("language-server-71", "compiler-90"),
                 ("batch-import-301", "postgres-424"), ("code-311", "llama-server-404")]:
        links.append((a, b, False))
    return links


# ----------------------------------------------------------------------------- projection
class Camera:
    def __init__(self, W, H_sky, beta0=12.0, half_fov=76.0):
        b = math.radians(beta0)
        self.f = np.array([math.cos(b), 0.0, math.sin(b)])
        self.u = np.array([-math.sin(b), 0.0, math.cos(b)])
        self.r = np.array([0.0, -1.0, 0.0])
        self.K = (W / 2) / math.tan(math.radians(half_fov) / 2)
        self.cx, self.cy = W / 2, H_sky * 0.5

    def vec(self, lam, beta):
        lam, beta = np.radians(lam), np.radians(beta)
        return np.stack([np.cos(beta) * np.cos(lam), np.cos(beta) * np.sin(lam), np.sin(beta)], -1)

    def project(self, lam, beta):
        v = self.vec(lam, beta)
        den = 1 + v @ self.f
        X = (v @ self.r) / den
        Y = (v @ self.u) / den
        return self.cx + X * self.K, self.cy - Y * self.K

    def unproject(self, px, py):
        X = (px - self.cx) / self.K
        Y = -(py - self.cy) / self.K
        s = X * X + Y * Y
        v = (2 * X[..., None] * self.r + 2 * Y[..., None] * self.u + (1 - s)[..., None] * self.f) / (1 + s)[..., None]
        return v


# ----------------------------------------------------------------------------- scene
class Scene:
    def __init__(self, W, H, pop, kernel, fit, lay, links):
        self.W, self.H = W, H
        self.sc = W / 1600.0
        sc = self.sc
        self.status_h = int(round(66 * sc)) if W >= 1200 else 50
        self.Hs = H - self.status_h
        self.pop, self.kernel, self.fit, self.lay, self.links = pop, kernel, fit, lay, links
        self.cam = Camera(W, self.Hs)
        cam = self.cam
        # sky positions
        for p in pop:
            lam, beta = lay["pos"][p["name"]]
            x, y = cam.project(lam, beta)
            p["sky"] = (float(x), float(y))
        base = S.sky(W, H, glow=(135, 55, 135))
        yy, xx = np.mgrid[0:H, 0:W].astype(np.float32)
        nx = (xx - W / 2) / (W / 2)
        horizon = self.Hs - (26 * sc) * (1 - nx ** 2)  # the dome rim arches across the bottom
        cove = np.exp(-np.clip(horizon - yy, 0, None) / (70 * sc))
        base += cove[..., None] * np.array([70, 42, 52], np.float32) * 0.55
        vig = 1 - 0.32 * np.clip(np.hypot(nx, (yy - H * 0.45) / (H * 0.6)) - 0.55, 0, None)
        base *= vig[..., None]
        rim = yy > horizon
        base[rim] = base[rim] * 0.15 + np.array([6, 5, 12], np.float32)
        self.base = base
        self.horizon = horizon
        self._milky_way()
        self._boundaries()
        self._graticule()
        self._hr_layout()
        self._names()

    # Milky Way: unresolved band of kernel threads. Brightness ~ count; texture decorative.
    def _milky_way(self):
        W, Hs, sc = self.W, self.Hs, self.sc
        yy, xx = np.mgrid[0:Hs, 0:W].astype(np.float32)
        v = self.cam.unproject(xx, yy)
        n = np.array([0.35, 0.55, -0.76])
        n /= np.linalg.norm(n)
        lat = np.degrees(np.arcsin(np.clip(v @ n, -1, 1)))
        # along-band coordinate for brightness variation (galactic centre glow on one side)
        e1 = np.cross(n, [0, 0, 1.0])
        e1 /= np.linalg.norm(e1)
        e2 = np.cross(n, e1)
        lon = np.degrees(np.arctan2(v @ e2, v @ e1))
        r = np.random.default_rng(11)

        def fbm(shape, scales):
            acc = np.zeros(shape, np.float32)
            for s, amp in scales:
                acc += amp * ndimage.gaussian_filter(r.standard_normal(shape).astype(np.float32), s * sc)
            return acc / max(1e-6, np.abs(acc).max())

        noise = fbm((Hs, W), [(70, 1.0), (28, 0.7), (10, 0.4), (3.5, 0.18), (1.2, 0.07)])
        dust = fbm((Hs, W), [(30, 1.0), (12, 0.55), (4, 0.25)])
        width = 9.0 + 2.5 * noise
        core = np.exp(-(lat / width) ** 2)
        wide = np.exp(-(lat / 22.0) ** 2) * 0.35
        bulge = 0.55 + 0.45 * np.exp(-((lon - 30) / 35.0) ** 2)
        lane = 1.0 - 0.75 * np.exp(-((lat - 1.5 + 2.0 * dust) / 2.6) ** 2) * np.clip(0.6 + dust, 0, 1)
        mw = (core * (0.75 + 0.5 * noise) * lane + wide) * bulge
        mw = np.clip(mw, 0, None)
        count = len(self.kernel)
        gain = 0.68 * (count / 58.0) ** 0.5
        col_core = np.array([235, 220, 205], np.float32)
        col_edge = np.array([120, 130, 190], np.float32)
        t = np.clip(core, 0, 1)[..., None]
        layer = np.zeros((self.H, W, 3), np.float32)
        layer[:Hs] = (mw[..., None] * (col_core * t + col_edge * (1 - t))) * gain * 0.42
        # unresolved points: one faint point per kernel thread inside the band
        pts = []
        for k in self.kernel:
            rr = rng_for(k)
            for _ in range(40):
                x, y = rr.random() * W, rr.random() * Hs
                vv = self.cam.unproject(np.array([x]), np.array([y]))[0]
                la = math.degrees(math.asin(max(-1, min(1, vv @ n))))
                if abs(la) < 7 and rr.random() < math.exp(-(la / 4) ** 2):
                    pts.append((x, y))
                    break
        self.kernel_pts = pts
        for x, y in pts:
            S.glow(layer, x, y, 0.8 * sc + 0.3, (240, 225, 210), 0.35)
        self.mw = layer

    def _boundaries(self):
        lay, cam = self.lay, self.cam
        owner, nl, nb = lay["owner"], lay["nl"], lay["nb"]
        edges = []
        # vertical edges: constant lambda between cells (i-1, j) and (i, j)
        for i in range(1, nl):
            for j in range(nb):
                if owner[i - 1, j] != owner[i, j] and min(owner[i - 1, j], owner[i, j]) >= 0:
                    edges.append(((i, j), (i, j + 1)))
        for i in range(nl):
            for j in range(1, nb):
                if owner[i, j - 1] != owner[i, j] and min(owner[i, j - 1], owner[i, j]) >= 0:
                    edges.append(((i, j), (i + 1, j)))
        adj = {}
        for a, b in edges:
            adj.setdefault(a, []).append(b)
            adj.setdefault(b, []).append(a)
        used = set()
        chains = []
        for a, b in edges:
            if (a, b) in used:
                continue
            chain = [a, b]
            used.add((a, b))
            used.add((b, a))
            for end in (0, 1):
                while True:
                    tip = chain[-1] if end == 0 else chain[0]
                    nxt = [c for c in adj[tip] if (tip, c) not in used]
                    if len(adj[tip]) != 2 or not nxt:
                        break
                    c = nxt[0]
                    used.add((tip, c))
                    used.add((c, tip))
                    if end == 0:
                        chain.append(c)
                    else:
                        chain.insert(0, c)
            chains.append(chain)
        polys = []
        for chain in chains:
            pts = []
            for a, b in zip(chain[:-1], chain[1:]):
                for s in np.linspace(0, 1, 6)[: -1]:
                    gi = a[0] + (b[0] - a[0]) * s
                    gj = a[1] + (b[1] - a[1]) * s
                    pts.append((L0 + gi * DL, B0 + gj * DB))
            b = chain[-1]
            pts.append((L0 + b[0] * DL, B0 + b[1] * DB))
            lam = np.array([q[0] for q in pts])
            bet = np.array([q[1] for q in pts])
            x, y = cam.project(lam, bet)
            polys.append(list(zip(x.tolist(), y.tolist())))
        self.bounds = polys

    def _graticule(self):
        cam = self.cam
        lines = []
        for lam in np.arange(-90, 91, 15):
            b = np.linspace(-60, 88, 120)
            x, y = cam.project(np.full_like(b, lam), b)
            lines.append(list(zip(x.tolist(), y.tolist())))
        for beta in np.arange(-50, 81, 10):
            l = np.linspace(-100, 100, 160)
            x, y = cam.project(l, np.full_like(l, beta))
            lines.append(list(zip(x.tolist(), y.tolist())))
        self.grat = lines

    def _names(self):
        lay, cam, sc = self.lay, self.cam, self.sc
        owner = lay["owner"]
        self.name_pos = {}
        stars = np.array([p["sky"] for p in self.pop])
        bright = np.array([sc * (6 + 2.2 * max(0.0, 4 - p["m"])) for p in self.pop])
        byname = {p["name"]: p["sky"] for p in self.pop}
        segs = [(byname[a], byname[b]) for a, b, same in self.links if same]
        fsz = int(round(15 * sc))
        placed = []
        for gi, g in enumerate(GROUPS):
            tw = 0.0
            f_big, f_small = S.font(fsz), S.font(int(round(fsz * 0.74)))
            for i, ch in enumerate(g):
                tw += (f_big if i == 0 else f_small).getlength(ch.upper()) + 0.32 * fsz
            th = fsz * 1.1
            cells = np.argwhere(owner == gi)
            cand = []
            for i, j in cells:
                for fl in (-0.25, 0.25):
                    for fb in (-0.25, 0.25):
                        cand.append((lay["lc"][i] + fl * DL, lay["bc"][j] + fb * DB))
            cand = np.array(cand)
            x, y = cam.project(cand[:, 0], cand[:, 1])
            mx, my = np.mean(x), np.mean(y)
            best = None
            for xi, yi in zip(x, y):
                x0, x1, y0, y1 = xi - tw / 2, xi + tw / 2, yi - th, yi + 0.3 * th
                if x0 < 12 * sc or x1 > self.W - 12 * sc or y0 < 10 * sc or y1 > self.Hs - 34 * sc:
                    continue
                # the whole label should sit inside the region
                inside = 0
                for fx in (0.0, 0.5, 1.0):
                    v = cam.unproject(np.array([x0 + fx * (x1 - x0)]), np.array([yi - th / 2]))[0]
                    lam, bet = math.degrees(math.atan2(v[1], v[0])), math.degrees(math.asin(v[2]))
                    ci, cj = int((lam - L0) // DL), int((bet - B0) // DB)
                    inside += owner[ci, cj] == gi
                pen = (3 - inside) * 400.0
                dx = np.clip(np.maximum(x0 - stars[:, 0], stars[:, 0] - x1), 0, None)
                dy = np.clip(np.maximum(y0 - stars[:, 1], stars[:, 1] - y1), 0, None)
                dist = np.hypot(dx, dy)
                pen += np.sum(np.clip(bright - dist, 0, None)) * 30
                for (ax, ay), (bx, by) in segs:
                    for k in np.linspace(0, 1, 9):
                        px, py = ax + (bx - ax) * k, ay + (by - ay) * k
                        if x0 - 3 < px < x1 + 3 and y0 - 3 < py < y1 + 3:
                            pen += 60
                for (qx0, qx1, qy0, qy1) in placed:
                    if x0 < qx1 and qx0 < x1 and y0 < qy1 and qy0 < y1:
                        pen += 2000
                pen += math.hypot(xi - mx, yi - my) * 0.6
                if best is None or pen < best[0]:
                    best = (pen, xi, yi, (x0, x1, y0, y1))
            if best is None:
                continue
            placed.append(best[3])
            self.name_pos[g] = (best[1], best[2])

    def _hr_layout(self):
        W, Hs, sc = self.W, self.Hs, self.sc
        self.px0, self.px1 = 150 * sc, W - 130 * sc
        self.py0, self.py1 = 92 * sc, Hs - 92 * sc
        self.lt_hot, self.lt_cool = math.log10(45000), math.log10(2250)
        self.m_top, self.m_bot = -4.0, 8.0
        for p in self.pop:
            p["hr"] = (self.hx(p["logT"]), self.hy(p["m"]))

    def hx(self, logT):
        return self.px0 + (self.lt_hot - logT) / (self.lt_hot - self.lt_cool) * (self.px1 - self.px0)

    def hy(self, m):
        return self.py0 + (m - self.m_top) / (self.m_bot - self.m_top) * (self.py1 - self.py0)


# ----------------------------------------------------------------------------- drawing
def star(canvas, x, y, m, rgb, mult, sc, size_gain=1.0):
    """Planetarium point-spread: flux = 10^(-0.4 (m - 7)); halo and core grow as flux^0.2."""
    L = 10 ** (-0.4 * (m - 7.0)) * mult
    if L > 60:  # wide soft bloom for the brightest stars
        S.glow(canvas, x, y, sc * (10 + 4.0 * L ** 0.2) * size_gain, rgb, min(0.16, 0.012 * L ** 0.3))
    halo_r = sc * (1.9 + 2.9 * L ** 0.21) * size_gain
    halo_s = min(0.95, 0.07 * L ** 0.42)
    S.glow(canvas, x, y, halo_r, rgb, halo_s)
    core_r = sc * (0.62 + 0.36 * L ** 0.2) * size_gain + 0.3
    core_s = min(2.2, 0.5 + 0.24 * L ** 0.3)
    white = tuple(0.4 * c + 0.6 * 255 for c in rgb)
    S.glow(canvas, x, y, core_r, white, core_s)
    return core_r


def ring(canvas, x, y, radius, thick, colour, strength):
    h, w, _ = canvas.shape
    R = int(radius + 3 * thick + 2)
    x0, x1 = max(int(x) - R, 0), min(int(x) + R + 1, w)
    y0, y1 = max(int(y) - R, 0), min(int(y) + R + 1, h)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1].astype(np.float32)
    d = np.hypot(xx - x, yy - y)
    ang = np.arctan2(yy - y, xx - x)
    lumpy = 0.75 + 0.25 * np.sin(ang * 7 + 1.3) * np.sin(ang * 3 - 0.4)
    g = np.exp(-((d - radius) / thick) ** 2) * strength * lumpy
    inner = np.exp(-(d / max(radius, 1)) ** 2 * 2.0) * strength * 0.18
    canvas[y0:y1, x0:x1] += (g + inner)[..., None] * np.array(colour, np.float32)


def render(sc_, t, u, labels_on=True, events=None, status_extra=None, ms_draw=1.0):
    """u: 0 = sky, 1 = HR diagram. events: dict with optional 'sn' (time since supernova) and
    'nova' (time since birth)."""
    W, H, Hs, sc = sc_.W, sc_.H, sc_.Hs, sc_.sc
    events = events or {}
    canvas = sc_.base.copy()
    sky_a = 1.0 - smoothstep(0.0, 0.62, u)
    hr_a = smoothstep(0.38, 1.0, u)

    # Milky Way and grid
    if sky_a > 0:
        canvas += sc_.mw * sky_a
        ink = Ink(W, H, 2)
        for ln in sc_.grat:
            ink.line(ln, 0.8 * sc, 255)
        over(canvas, ink.mask(), (110, 140, 200), 0.10 * sky_a)
        ink = Ink(W, H, 3)
        for poly in sc_.bounds:
            for d in dashes(poly, 5 * sc, 4 * sc):
                ink.line(d, 1.0 * sc, 255)
        over(canvas, ink.mask(), (196, 120, 96), 0.42 * sky_a)

    # star positions
    pos = {}
    for p in sc_.pop:
        if p.get("gone"):
            continue
        sx, sy = p["sky"]
        hx, hy = p["hr"]
        lag = h01(p["name"], "lag") * 0.28
        k = ease(min(max((u - lag * 0.5) / (1 - 0.28 * 0.5), 0), 1)) if 0 < u < 1 else u
        # curved flight path: bulge perpendicular to the straight line
        mx, my = (sx + hx) / 2, (sy + hy) / 2
        dx, dy = hx - sx, hy - sy
        bend = (h01(p["name"], "bend") - 0.5) * 0.5
        cx_, cy_ = mx - dy * bend, my + dx * bend
        x = (1 - k) ** 2 * sx + 2 * (1 - k) * k * cx_ + k * k * hx
        y = (1 - k) ** 2 * sy + 2 * (1 - k) * k * cy_ + k * k * hy
        pos[p["name"]] = (x, y)

    # constellation stick figures, with gaps around stars
    if sky_a > 0:
        inner = Ink(W, H, 3)
        cross = Ink(W, H, 3)
        gap = {p["name"]: sc * (4.5 + 1.6 * max(0.0, 3.5 - p["m"])) for p in sc_.pop}
        for a, b, same in sc_.links:
            if a not in pos or b not in pos:
                continue
            (x0, y0), (x1, y1) = pos[a], pos[b]
            d = math.hypot(x1 - x0, y1 - y0)
            if d < gap[a] + gap[b] + 2:
                continue
            ux, uy = (x1 - x0) / d, (y1 - y0) / d
            seg = [(x0 + ux * gap[a], y0 + uy * gap[a]), (x1 - ux * gap[b], y1 - uy * gap[b])]
            if same:
                inner.line(seg, 1.15 * sc, 255)
            elif d < 420 * sc:
                for dsh in dashes(seg, 3 * sc, 5 * sc):
                    cross.line(dsh, 1.0 * sc, 255)
        over(canvas, inner.mask(), (96, 150, 225), 0.62 * sky_a)
        over(canvas, cross.mask(), (96, 150, 225), 0.30 * sky_a)

    # HR axes, grid, main sequence
    img_layers = []
    if hr_a > 0:
        a, b = sc_.fit
        ink = Ink(W, H, 3)
        for letter, lo, hi in CLASSES[:-1]:
            x = sc_.hx(math.log10(lo))
            ink.line([(x, sc_.py0 - 18 * sc), (x, sc_.py1)], 1.0 * sc, 255)
        over(canvas, ink.mask(), (150, 160, 200), 0.18 * hr_a)
        ink = Ink(W, H, 3)
        for mm in range(-4, 9, 2):
            y = sc_.hy(mm)
            ink.line([(sc_.px0, y), (sc_.px1, y)], 0.8 * sc, 255)
        over(canvas, ink.mask(), (150, 160, 200), 0.07 * hr_a)
        ink = Ink(W, H, 3)
        ink.line([(sc_.px0, sc_.py0 - 18 * sc), (sc_.px0, sc_.py1), (sc_.px1, sc_.py1)], 1.2 * sc, 255)
        over(canvas, ink.mask(), (180, 188, 215), 0.55 * hr_a)
        # spectral class bands: faint tint of each class colour along the top
        # main sequence: glowing band plus the fitted line, drawn in from the cool end
        lts = np.linspace(sc_.lt_cool, sc_.lt_hot, 200)
        n_draw = max(2, int(len(lts) * ms_draw))
        lts = lts[:n_draw]
        lts = lts[(a + b * lts >= sc_.m_top + 0.15) & (a + b * lts <= sc_.m_bot)]
        xs = [sc_.hx(v) for v in lts]
        ys = [sc_.hy(a + b * v) for v in lts]
        band = Ink(W, H, 2)
        band.line(list(zip(xs, ys)), 26 * sc, 255)
        bm = ndimage.gaussian_filter(band.mask(), 9 * sc)
        add(canvas, bm, (120, 150, 230), 0.22 * hr_a)
        ink = Ink(W, H, 3)
        for d in dashes(list(zip(xs, ys)), 9 * sc, 5 * sc):
            ink.line(d, 1.3 * sc, 255)
        over(canvas, ink.mask(), (150, 185, 255), 0.6 * hr_a)

    # supernova remnant
    sn = events.get("sn")
    if sn is not None and sn[0] >= 0:
        dt, (x, y) = sn
        flash = math.exp(-dt / 0.28)
        S.glow(canvas, x, y, (16 + 26 * (1 - math.exp(-dt / 0.15))) * sc, (215, 225, 255), 2.2 * flash)
        S.glow(canvas, x, y, 5 * sc, (255, 255, 255), 3.0 * flash)
        rad = sc * (8 + 95 * (1 - math.exp(-dt / 1.1)))
        fade = math.exp(-dt / 1.6)
        mix = min(1.0, dt / 1.5)
        col = (200 * (1 - mix) + 255 * mix, 215 * (1 - mix) + 95 * mix, 255 * (1 - mix) + 140 * mix)
        ring(canvas, x, y, rad, (2.0 + 2.5 * min(dt, 1.5)) * sc, col, 1.1 * fade)

    # stars
    core = {}
    for p in sc_.pop:
        if p["name"] not in pos:
            continue
        x, y = pos[p["name"]]
        n = 0.0
        for q in range(3):
            f = 0.9 + 2.6 * h01(p["name"], f"f{q}")
            ph = 6.283 * h01(p["name"], f"p{q}")
            n += math.sin(6.283 * f * t + ph)
        n /= 1.7
        mult = max(0.12, 1.0 + p["twinkle"] * n * (1 - 0.7 * u))
        if p["name"] == events.get("nova_name") and events.get("nova") is not None:
            dt = events["nova"]
            mult *= min(1.0, 0.15 + dt / 0.5)
            S.glow(canvas, x, y, (9 + 20 * min(dt, 1)) * sc, (225, 230, 255), 1.6 * math.exp(-dt / 0.35))
        core[p["name"]] = star(canvas, x, y, p["m"], p["rgb"], mult, sc, 1.0 - 0.15 * u)

    img = S.to_image(canvas)
    draw = ImageDraw.Draw(img)

    # constellation names
    if sky_a > 0.02:
        nm = Image.new("RGBA", img.size, (0, 0, 0, 0))
        nd = ImageDraw.Draw(nm)
        for g, (x, y) in sc_.name_pos.items():
            smallcaps(nd, x, y, g, int(round(15 * sc)) if sc > 0.7 else 11, (150, 180, 230, 255),
                      tracking=0.32, shadow=False)
        arr = np.asarray(nm).astype(np.float32)
        arr[..., 3] *= 0.62 * sky_a
        img = Image.alpha_composite(img.convert("RGBA"), Image.fromarray(arr.astype(np.uint8))).convert("RGB")
        draw = ImageDraw.Draw(img)

    # HR text
    if hr_a > 0.02:
        tx = Image.new("RGBA", img.size, (0, 0, 0, 0))
        td = ImageDraw.Draw(tx)
        fs = max(10, int(round(13 * sc)))
        big = max(13, int(round(22 * sc)))
        for letter, lo, hi in CLASSES:
            hi_ = min(hi, 45000)
            lo_ = max(lo, 2250)
            x = (sc_.hx(math.log10(lo_)) + sc_.hx(math.log10(hi_))) / 2
            col = tuple(int(c) for c in bb_colour(math.sqrt(lo_ * hi_)))
            td.text((x, sc_.py0 - 30 * sc), letter, font=S.font(big, S.FONT_MONO_BOLD), fill=col + (255,),
                    anchor="ms")
        for T, lab in [(40000, "40000 K"), (20000, "20000 K"), (10000, "10000 K"), (7000, "7000 K"),
                       (5000, "5000 K"), (3500, "3500 K"), (2400, "2400 K")]:
            x = sc_.hx(math.log10(T))
            cpu = 5 * ((T / 2400.0) ** (1 / 0.47) - 1)
            cl = f"{cpu:.0f}% cpu" if cpu >= 10 else f"{cpu:.1f}% cpu"
            td.line([(x, sc_.py1), (x, sc_.py1 + 5 * sc)], fill=(180, 188, 215, 200), width=1)
            td.text((x, sc_.py1 + 9 * sc), lab, font=S.font(fs), fill=S.TEXT + (230,), anchor="mt")
            td.text((x, sc_.py1 + 9 * sc + fs * 1.3), cl, font=S.font(fs - 1), fill=S.DIM + (230,), anchor="mt")
        for mm in range(-4, 9, 2):
            y = sc_.hy(mm)
            td.text((sc_.px0 - 10 * sc, y), f"{mm:+d}" if mm else "0", font=S.font(fs), fill=S.TEXT + (230,),
                    anchor="rm")
            mem = 10 ** (-mm / 2.5) * GIB
            ml = f"{mem / GIB:.0f} GiB" if mem >= GIB else f"{mem / MIB:.0f} MiB"
            if mem < MIB:
                ml = f"{mem / 1024:.0f} KiB"
            td.text((sc_.px1 + 10 * sc, y), ml, font=S.font(fs - 1), fill=S.DIM + (230,), anchor="lm")
        td.text((sc_.px0 - 10 * sc, sc_.py0 - 30 * sc), "m", font=S.font(fs), fill=S.DIM + (230,), anchor="rs")
        td.text((sc_.px1 + 10 * sc, sc_.py0 - 30 * sc), "memory", font=S.font(fs - 1), fill=S.DIM + (230,),
                anchor="ls")
        td.text(((sc_.px0 + sc_.px1) / 2, sc_.py1 + 9 * sc + fs * 2.7), "surface temperature   (hot on the left)",
                font=S.font(fs - 1), fill=S.DIM + (230,), anchor="mt")
        smallcaps(td, sc_.px0 + 14 * sc, sc_.py0 - 52 * sc, "Hertzsprung-Russell diagram of this machine",
                  max(11, int(round(15 * sc))), (200, 206, 225, 235), tracking=0.18, shadow=False, anchor="l")
        # region names, placed relative to the fit
        a, b = sc_.fit
        lab_s = max(10, int(round(14 * sc)))

        def region(text, lt, dm, rot=0.0):
            x, y = sc_.hx(lt), sc_.hy(a + b * lt + dm)
            lay = Image.new("RGBA", (int(400 * sc) + 40, int(40 * sc) + 20), (0, 0, 0, 0))
            ld = ImageDraw.Draw(lay)
            smallcaps(ld, lay.width / 2, lay.height / 2 + 5 * sc, text, lab_s, (165, 185, 235, 200),
                      tracking=0.4, shadow=False)
            lay = lay.rotate(rot, resample=Image.BICUBIC, expand=True)
            tx.alpha_composite(lay, (int(x - lay.width / 2), int(y - lay.height / 2)))

        p0 = (sc_.hx(4.2), sc_.hy(a + b * 4.2))
        p1 = (sc_.hx(4.0), sc_.hy(a + b * 4.0))
        rot = math.degrees(math.atan2(-(p1[1] - p0[1]), p1[0] - p0[0]))
        if ms_draw > 0.9:
            region("main sequence", 4.10, 1.25, rot)
            region("giants", 3.66, -3.2)
            region("supergiants", 3.50, -6.6)
            region("white dwarfs", 4.20, 5.8)
        arr = np.asarray(tx).astype(np.float32)
        arr[..., 3] *= hr_a
        img = Image.alpha_composite(img.convert("RGBA"), Image.fromarray(arr.astype(np.uint8))).convert("RGB")
        draw = ImageDraw.Draw(img)

    # star labels
    if labels_on:
        lab_img = Image.new("RGBA", img.size, (0, 0, 0, 0))
        main_draw, draw = draw, ImageDraw.Draw(lab_img)
        fs = max(10, int(round(14 * sc)))
        for name, (dxs, dys), (dxh, dyh) in LABELS:
            if name not in pos:
                continue
            p = next(q for q in sc_.pop if q["name"] == name)
            x, y = pos[name]
            dx = dxs * (1 - u) + dxh * u
            dy = dys * (1 - u) + dyh * u
            r = core.get(name, 3) + 6 * sc
            text = f"{name} {p['cls']}"
            tw = S.font(fs).getlength(text)
            if dx >= 0 and x + r + dx * sc + tw > W - 8 * sc:
                dx = -abs(dx)
            elif dx < 0 and x - r + dx * sc - tw < 8 * sc:
                dx = abs(dx)
            anchor = "lm" if dx >= 0 else "rm"
            vis = 1.0 - 4 * u * (1 - u)  # fade out mid-flight
            if vis < 0.15:
                continue
            col = tuple(S.TEXT) + (int(255 * vis),)
            lx, ly = x + math.copysign(r, dx) + dx * sc, y + dy * sc
            draw.text((lx + 1, ly + 1), text, font=S.font(fs), fill=(0, 0, 0, int(255 * vis)), anchor=anchor)
            draw.text((lx, ly), text, font=S.font(fs), fill=col, anchor=anchor)
        img = Image.alpha_composite(img.convert("RGBA"), lab_img).convert("RGB")

    return img


# (name, sky offset, HR offset) in 1600-px units; sign of dx picks the side
LABELS = [
    ("postgres-41", (6, -2), (8, -12)),
    ("compiler-90", (6, 0), (6, -12)),
    ("ffmpeg-999", (6, 0), (6, 0)),
    ("java-elastic-77", (-6, 0), (-6, 0)),
    ("llama-server-404", (6, 0), (6, 0)),
]


def status_lines(sc_, mode, extra=None, t=0.0, compact=False):
    pop = [p for p in sc_.pop if not p.get("gone")]
    n_proc = len(pop) + len(sc_.kernel)
    a, b = sc_.fit
    ram = sum(p["mem"] for p in pop) / GIB
    cpu = sum(p["cpu"] for p in pop) / 100
    l1 = (f"ISOTOP / HR / DEMO   {n_proc} processes | {len(pop)} stars | {len(sc_.kernel)} kernel threads in the "
          f"Milky Way | {len(GROUPS)} constellations (cgroups) | {cpu:.1f}/48 CPU cores | RAM {ram:.1f} GiB / 64.0 GiB")
    if mode == "sky":
        l2 = ("colour = blackbody T = 2400 K x (1 + cpu/5)^0.47 | size, brightness = memory, m = -2.5 log10(mem / 1 GiB) "
              "| twinkle = CPU variation over 10 s")
        l3 = ("lines = socket links | dashed = cgroup boundaries on the RA/Dec grid | band = kernel threads | "
              f"d diagram | click inspect | t={t:.1f}s")
    else:
        l2 = (f"x = temperature from CPU, hot on the left | y = magnitude from memory | main sequence: Theil-Sen fit "
              f"m = {a:.1f} - {abs(b):.2f} log T | class from T, luminosity class from residual")
        l3 = f"d sky | classes: Ia Ib II III IV V VI below the fit, D = A or hotter, 2.75 mag below | t={t:.1f}s"
    if compact:
        l1 = (f"ISOTOP / HR / DEMO   {n_proc} processes | {len(pop)} stars | {len(sc_.kernel)} kernel threads "
              f"(Milky Way) | {len(GROUPS)} cgroups")
        if mode == "sky":
            l2 = "colour = blackbody T from cpu | size = memory, m = -2.5 log10(mem / 1 GiB) | twinkle = CPU variation"
            l3 = f"lines = socket links | dashed = cgroup boundaries | d diagram | t={t:.1f}s"
        else:
            l2 = "x = temperature from cpu, hot on the left | y = magnitude from memory | dashed = fitted main sequence"
            l3 = f"spectral class from T, luminosity class from residual to the fit | d sky | t={t:.1f}s"
    if extra:
        l1 = l1.split(" | 13 constellations")[0].split(" | 13 cgroups")[0] + "   " + extra
    return [l1, l2, l3]


def main():
    import time
    t0 = time.time()
    pop, kernel, fit = build_population()
    for name in ["postgres-41", "compiler-90", "ffmpeg-999", "java-elastic-77", "llama-server-404", "browser-512",
                 "gzip-303", "worker-134", "batch-import-301", "logstash-79", "gnome-shell-1500"]:
        p = next(q for q in pop if q["name"] == name)
        print(f"{name:20s} cpu {p['cpu']:7.1f} mem {p['mem'] / MIB:7.0f} MiB T {p['T']:6.0f} m {p['m']:5.2f} "
              f"resid {p['resid']:5.2f} {p['cls']}")
    print("fit", fit, "classes:", sorted(set(p["cls"].split()[-1] if " " in p["cls"] else p["cls"] for p in pop)))
    for T in [2400, 3000, 5800, 6500, 10000, 20000, 40000]:
        print(T, [round(c) for c in bb_colour(T)])
    lay = sky_layout(pop, Camera(1600, 834), 1600, 834)
    links = socket_links(pop, lay["pos"])
    print("layout", time.time() - t0)
    mode = sys.argv[1] if len(sys.argv) > 1 else "still"
    if mode in ("still", "both"):
        sc_ = Scene(1600, 900, pop, kernel, fit, lay, links)
        print("scene", time.time() - t0)
        img = render(sc_, 3.3, 0.0)
        S.status(img, status_lines(sc_, "sky", t=3.3), size=13)
        print(S.save_still(img, "hr"))
        img = render(sc_, 3.3, 1.0)
        S.status(img, status_lines(sc_, "hr", t=3.3), size=13)
        img.save(f"{S.OUT}/hr-diagram.png")
        print("still", time.time() - t0)
    if mode in ("anim", "both"):
        animate(pop, kernel, fit, lay, links)
        print("anim", time.time() - t0)


def animate(pop, kernel, fit, lay, links):
    sc_ = Scene(960, 540, pop, kernel, fit, lay, links)
    fps = 12
    n = 128
    sn_name = "batch-import-301"
    snp = next(p for p in sc_.pop if p["name"] == sn_name)
    t_sn = 1.0
    t_fly1, t_fly2 = 3.4, 7.6
    t_nova = 9.9
    frames = []
    for i in range(n):
        t = i / fps
        if t < t_fly1:
            u = 0.0
        elif t < t_fly1 + 1.5:
            u = (t - t_fly1) / 1.5
        elif t < t_fly2:
            u = 1.0
        elif t < t_fly2 + 1.5:
            u = 1.0 - (t - t_fly2) / 1.5
        else:
            u = 0.0
        gone = t_sn <= t < t_nova
        snp["gone"] = gone
        events = {}
        extra = None
        if t >= t_sn and t < t_fly1 + 0.6:
            events["sn"] = (t - t_sn, snp["sky"])
            extra = "batch-import-301 exited holding 2.1 GiB: supernova"
        if t >= t_nova:
            events["nova"] = t - t_nova
            events["nova_name"] = sn_name
            extra = "batch-import-302 started (backup.service restarted): nova"
        if t_fly1 + 1.5 <= t < t_fly2:
            ms = min(1.0, (t - t_fly1 - 1.5) / 1.0)
        elif t_fly1 <= t < t_fly1 + 1.5:
            ms = 0.0
        else:
            ms = 1.0
        img = render(sc_, t, u, events=events, ms_draw=ms)
        mode = "hr" if u > 0.5 else "sky"
        if extra is None and u > 0.5:
            extra = "diagram: compiler-90 A0 V, postgres-41 K3 III, ffmpeg-999 DA"
        S.status(img, status_lines(sc_, mode, extra, t=t, compact=True), size=11)
        frames.append(img)
    snp["gone"] = False
    path, size = S.save_frames(frames, "hr", fps=fps)
    import gifpack
    path, size = gifpack.pack("hr", fps=fps, width=720, colors=80)
    print(path, size)
    sheet = Image.new("RGB", (960 * 2, 540 * 2))
    for k, fi in enumerate([6, 26, 60, 100]):
        sheet.paste(frames[fi], ((k % 2) * 960, (k // 2) * 540))
    sheet.save(f"{S.OUT}/hr-sheet.png")


if __name__ == "__main__":
    main()
