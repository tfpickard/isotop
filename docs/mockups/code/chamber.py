"""Mockup of isotop's `chamber` view: a bubble chamber photograph that keeps developing.

Real physics in the mockup:
- Every busy process gyrates around its fixed home (the guiding centre) with Larmor radius
  r = p / (|q| B), p ~ smoothed CPU, charge sign from the utime/stime split.
- Speed along the track is beta = p / sqrt(p^2 + m^2) with m ~ memory, so the gyration
  frequency is omega = beta c / r; bubble density per unit length ~ 1 / beta^2 (clamped).
- A process whose CPU decays spirals in because p (and so r) falls; nothing is scripted
  beyond the CPU curve.
- Bubbles are only formed at each 1 s expansion, along the last second of motion; they grow
  over 200 ms and fade over a few seconds.
- A fork puts the child's track at the parent's position (a V0 when the parent is neutral);
  an exit ends the track in delta rays, one per thread, all curling the electron way.
- Beam: one straight track per K timer interrupts; cosmics: one per K other interrupts.
Decorative: film grain, fiducial crosses, the window rim and the frame counter.

usage: python3 -I chamber.py [still|anim|both]
"""
import math
import os
import sys
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

import gifpack
import isostyle as S

TYPE = "/usr/share/fonts/truetype/freefont/FreeMonoBold.ttf"
BASE_W, BASE_H = 1600, 900
CX, CY, R = 800.0, 420.0, 398.0          # chamber window, in 1600x900 pixels
K_R = 104.0                              # Larmor radius per core of CPU (pixels, B = 1)
C_VIS = 60.0                            # visual speed of light, px/s
M_PER_GIB = 1.20                         # mass per GiB of resident memory
DENSITY = 0.38                           # bubbles per px at beta = 1
DENSITY_CLAMP = 8.0                     # 1/beta^2 clamp
NEUTRAL_CPU = 0.10                       # below this a process is neutral and invisible
DT = 1.0 / 240.0
K_HOME = 0.6                             # guiding-centre relaxation to home, 1/s
BEAM_DENSITY = 0.17                      # beam and cosmic bubbles per px (visual constant)


def smooth_step(t, t0, t1):
    x = np.clip((t - t0) / (t1 - t0), 0, 1)
    return x * x * (3 - 2 * x)


class Proc:
    def __init__(self, name, cpu, mem, user, threads, home=None, born=0.0, died=None,
                 label=None, label_dx=14, label_dy=-10):
        self.name, self.cpu_fn, self.mem, self.user, self.threads = name, cpu, mem, user, threads
        self.home = home
        self.born, self.died = born, died
        self.label = label
        self.label_angle = label_dx      # fixed direction from home: labels never jump
        self.phi = 0.0
        self.offset = np.zeros(2)
        self.path = []       # (t, x, y, density)
        self.alive = False
        self.charged = False

    @property
    def sign(self):
        return 1.0 if self.user >= 0.5 else -1.0

    def cpu(self, t):
        return self.cpu_fn(t) if callable(self.cpu_fn) else self.cpu_fn

    def state(self, t):
        p = max(self.cpu(t), 1e-6)
        m = M_PER_GIB * self.mem + 0.02
        beta = p / math.sqrt(p * p + m * m)
        r = K_R * p
        return p, beta, r


def jitter(seed, base, amount=0.06, period=2.3):
    """Smoothed CPU noise: a few incommensurate sines seeded from process identity."""
    rng = np.random.default_rng(seed)
    ph = rng.random(3) * 6.28
    fr = (rng.random(3) * 0.6 + 0.7) / period

    def f(t):
        n = sum(math.sin(6.283 * fr[i] * t + ph[i]) for i in range(3)) / 3
        return base * (1 + amount * n)
    return f


def phyllotaxis(n, radius):
    golden = math.pi * (3 - math.sqrt(5))
    pts = []
    for i in range(n):
        rr = radius * math.sqrt((i + 0.5) / n)
        a = i * golden
        pts.append((CX + rr * math.cos(a), CY + rr * math.sin(a)))
    return pts


class Chamber:
    def __init__(self, events):
        self.events = events
        self.rng = np.random.default_rng(7)
        ev = events

        def home(dx, dy):
            return np.array([CX + dx, CY + dy], float)

        spiral_t0, spiral_tau = ev["spiral"]

        def decaying(t):
            if t < spiral_t0:
                return 1.0 * (1 + 0.04 * math.sin(t * 1.7))
            return 1.0 * math.exp(-(t - spiral_t0) / spiral_tau)

        P = Proc
        procs = [
            # name, cpu (cores), mem GiB, user fraction, threads, home (stable layout slot)
            P("compiler-90", jitter(90, 1.62), 1.2, 0.93, 12, home(-120, 10), label="compiler-90 +",
              label_dx=-2.55),
            P("worker-134", jitter(134, 2.15, 0.04), 0.9, 0.86, 16, home(120, 70)),
            P("browser-512", jitter(512, 0.92), 3.1, 0.72, 48, home(250, -60)),
            P("postgres-41", jitter(41, 0.46), 2.2, 0.62, 9, home(-10, 60), label="postgres-41 +",
              label_dx=0.35),
            P("java-415", jitter(415, 1.12), 2.6, 0.9, 64, home(40, 240)),
            P("node-301", jitter(301, 0.72, 0.2, 1.8), 0.5, 0.82, 11, home(-110, 300)),
            P("python-233", jitter(233, 0.52, 0.3, 1.4), 0.3, 0.77, 3, home(100, -110)),
            P("gnome-shell-88", jitter(88, 0.42, 0.3, 1.2), 0.8, 0.8, 22, home(-90, -120)),
            P("nginx-128", jitter(128, 0.09), 0.1, 0.30, 8, home(270, 220)),
            P("redis-207", jitter(207, 0.31, 0.3, 1.5), 0.6, 0.35, 4, home(-300, 30)),
            P("browser-tab-611", jitter(611, 0.08), 0.6, 0.8, 21, home(170, -300)),
            P("kworker/3", jitter(3, 0.26, 0.3, 1.0), 0.0, 0.02, 1, home(60, 150), label="kworker/3 -",
              label_dx=0.0),
            P("ksoftirqd/2", jitter(2, 0.15, 0.35, 0.9), 0.0, 0.0, 1, home(-230, 270)),
            P("language-server-71", decaying, 0.12, 0.88, 14, home(-250, 140),
              label="language-server-71 +", label_dx=math.pi),
            # Neutral: below the CPU threshold, no track at all.
            P("journald-310", jitter(310, 0.08), 0.2, 0.25, 2, home(-40, -40)),
            P("pipewire-95", jitter(95, 0.08), 0.05, 0.7, 4, home(200, 120)),
            P("dockerd-77", jitter(77, 0.06), 0.15, 0.2, 20, home(-200, -200)),
            P("kswapd0", jitter(5, 0.05), 0.0, 0.0, 1, home(20, -250)),
            P("rcu_preempt", jitter(9, 0.06), 0.0, 0.0, 1, home(-60, 200)),
        ]
        for pr in procs:
            pr.phi = (zlib.crc32(pr.name.encode()) % 6283) / 1000.0
        procs[0].phi = 4.6
        procs[0].label_pick = 0.45
        procs[1].phi = 0.6
        exit_t, exit_threads = ev["exit"]
        runner = P("test-runner-288", jitter(288, 1.3), 0.4, 0.9, exit_threads, home(160, 130),
                   died=exit_t)
        runner.phi = ev.get("runner_phi", 5.4)
        procs.append(runner)
        self.procs = procs
        # The fork: shell-170 is neutral (idle) and forks a pipeline, find | grep. Both children
        # start at the shell's position: a V0, a vertex from nowhere.
        fork_t = ev["fork"]
        vertex = home(*ev.get("vertex", (-240, -40)))
        self.fork_vertex = vertex
        self.fork_t = fork_t
        heading0 = ev.get("fork_heading", 0.05)
        for name, cpu, user, heading, lab in (
                ("find-4471", 1.05, 0.08, heading0 + 0.30, "find-4471 -"),
                ("grep-4472", 1.20, 0.94, heading0 - 0.30, "grep-4472 +")):
            child = P(name, jitter(zlib.crc32(name.encode()) % 1000, cpu), 0.01, user, 1, None,
                      born=fork_t)
            # The layout gives a newborn its home next to the parent: here, the guiding centre
            # whose circle passes through the parent's position with the child's heading.
            p, beta, r = child.state(fork_t)
            d = np.array([math.cos(heading), math.sin(heading)])
            # Motion is phi' = -s omega (screen coords, B out of the screen), so the tangent is
            # (s sin phi, -s cos phi); solve for the phase whose tangent is the heading.
            s = child.sign
            child.phi = math.atan2(s * d[0], -s * d[1])
            child.home = vertex - r * np.array([math.cos(child.phi), math.sin(child.phi)])
            procs.append(child)
        self.bubbles = []     # arrays of x, y, birth, size
        self.exits = []

    # ------------------------------------------------------------------ simulation
    def expand(self, tk):
        """One expansion: bubbles along the last second of every charged track, a beam
        bundle and cosmics, and delta rays for processes that died during the second."""
        rng = self.rng
        new = []
        for pr in self.procs:
            seg = [q for q in pr.path if tk - 1.0 < q[0] <= tk]
            if len(seg) < 2:
                continue
            pts = np.array([[q[1], q[2]] for q in seg])
            dens = np.array([q[3] for q in seg])
            new.append(self.along(pts, dens, tk))
            pr.head = pts[-1]
            pr.head_dir = pts[-1] - pts[-2]
        # Beam: one straight track per K LOC interrupts, entering from the left.
        loc = 16240 + 900 * math.sin(tk * 0.9)
        nbeam = int(round(loc / 5400))
        y0 = CY - 175
        for i in range(nbeam):
            # The beam is a narrow bundle; each expansion catches a fresh set of particles.
            y = y0 + 40 + rng.normal(0, 62)
            slope = rng.normal(0.0, 0.012)
            xs = np.linspace(CX - R - 5, CX + R + 5, 400)
            # A very stiff positive track: radius ~ 9000 px.
            ys = y + slope * (xs - (CX - R)) + (xs - (CX - R)) ** 2 / (2 * 9000.0)
            pts = np.stack([xs, ys], 1)
            new.append(self.along(pts, np.full(len(pts), BEAM_DENSITY), tk))
        # Cosmics: one random straight track per K other interrupts.
        for _ in range(rng.poisson(0.5)):
            a = rng.uniform(0, math.pi)
            off = rng.uniform(-R * 0.8, R * 0.8)
            d = np.array([math.cos(a), math.sin(a)])
            n = np.array([-d[1], d[0]])
            c = np.array([CX, CY]) + n * off
            s = np.linspace(-R * 1.1, R * 1.1, 300)
            pts = c + s[:, None] * d
            new.append(self.along(pts, np.full(len(pts), 0.09), tk))
        # Exits: delta rays, one per thread at death, all curling the electron way.
        for pr, te, pos in self.exits:
            if tk - 1.0 < te <= tk:
                n = pr.threads
                for k in range(n):
                    # A knock-on electron per thread: it leaves the death point outward, then
                    # curls ever tighter (electron sense) as it slows, getting denser.
                    theta = 2 * math.pi * (k + rng.uniform(-0.4, 0.4)) / n + 0.4
                    length = rng.uniform(28, 130) if k else 150
                    r0 = rng.uniform(12, 52)
                    m = 240
                    ds = length / m
                    pts = [pos.copy()]
                    dens = [0.3]
                    x = pos.copy()
                    for j in range(m):
                        f = j / m
                        r = r0 * (1 - 0.9 * f) + 2.5
                        theta += ds / r          # negative charge: the opposite turn to +
                        x = x + ds * np.array([math.cos(theta), math.sin(theta)])
                        pts.append(x.copy())
                        dens.append(0.32 + 0.9 * f * f)
                    new.append(self.along(np.array(pts), np.array(dens), tk))
                self.death_marks = getattr(self, "death_marks", []) + [(pr, pos, te)]
        if new:
            self.bubbles.append(np.concatenate(new, 0))

    def along(self, pts, dens, tk):
        rng = self.rng
        seg = np.linalg.norm(np.diff(pts, axis=0), axis=1)
        cum = np.concatenate([[0], np.cumsum(seg)])
        dmid = np.concatenate([[dens[0]], 0.5 * (dens[1:] + dens[:-1])])
        mass = np.concatenate([[0], np.cumsum(seg * dmid[1:])])
        n = rng.poisson(mass[-1])
        if n == 0:
            return np.zeros((0, 4))
        u = np.sort(rng.uniform(0, mass[-1], n))
        idx = np.clip(np.searchsorted(mass, u) - 1, 0, len(pts) - 2)
        f = (u - mass[idx]) / np.maximum(mass[idx + 1] - mass[idx], 1e-9)
        xy = pts[idx] + (pts[idx + 1] - pts[idx]) * f[:, None]
        xy += rng.normal(0, 0.35, xy.shape)    # bubble scatter about the true track
        size = rng.choice([0.0, 1.0], n, p=[0.7, 0.3])
        local = dmid[idx + 1]
        # Dense tracks also get bigger bubbles (they merge into a continuous line).
        size = np.where(local > DENSITY * 5, 1.0, size)
        return np.stack([xy[:, 0], xy[:, 1], np.full(n, tk), size], 1)


class Sim(Chamber):
    def run(self, t_end):
        t = 0.0
        next_expansion = 1.0
        while t < t_end - 1e-9:
            self.time = t
            for pr in self.procs:
                alive = t >= pr.born and (pr.died is None or t < pr.died)
                if not alive:
                    if pr.alive and pr.died is not None and t >= pr.died:
                        self.exits.append((pr, t, pr.last_pos.copy()))
                    pr.alive = False
                    continue
                pr.alive = True
                p, beta, r = pr.state(t)
                pr.charged = p >= NEUTRAL_CPU
                if not pr.charged:
                    continue
                v = C_VIS * beta
                s = pr.sign
                if getattr(pr, "x", None) is None:
                    # Start on the Larmor circle about home, moving along its tangent.
                    pr.x = pr.home + r * np.array([math.cos(pr.phi), math.sin(pr.phi)])
                    pr.theta = math.atan2(-s * math.cos(pr.phi), s * math.sin(pr.phi))
                # Integrate the track itself: curvature 1/r, turning the way the charge says.
                # CPU changes bend the track smoothly instead of moving it radially.
                pr.theta += -s * (v / r) * DT
                heading = np.array([math.cos(pr.theta), math.sin(pr.theta)])
                pr.x = pr.x + v * heading * DT
                # The guiding centre (centre of curvature) relaxes back to the stable home.
                centre = pr.x - s * r * np.array([-heading[1], heading[0]])
                pr.x = pr.x + (pr.home - centre) * K_HOME * DT
                pos = pr.x
                pr.last_pos = pos
                dens = DENSITY * min(1.0 / (beta * beta), DENSITY_CLAMP)
                pr.path.append((t, pos[0], pos[1], dens))
            t += DT
            if t >= next_expansion - 1e-9:
                self.expand(next_expansion)
                next_expansion += 1.0
        self.time = t_end


# ---------------------------------------------------------------------- rendering
_BASE = {}


def base_layer(w, h):
    if (w, h) in _BASE:
        return _BASE[(w, h)]
    s = w / BASE_W
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    cx, cy, r = CX * s, CY * s, R * s
    d = np.hypot(xx - cx, yy - cy)
    inside = np.clip(r - d + 0.5, 0, 1)
    # Dark-field chamber: near-black liquid, a little light scattered towards the rim.
    rho = d / r
    liquid = 13 + 9 * rho ** 3 + 4 * np.exp(-((xx - cx * 0.8) ** 2 + (yy - cy * 0.75) ** 2) / (2 * (r * 0.6) ** 2))
    # The steel flange around the glass, with bolts (decorative).
    ring = np.clip(1 - np.abs(d - (r + 13 * s)) / (12 * s), 0, 1)
    flange = 6 + 26 * ring ** 0.6 * (0.7 + 0.3 * np.cos(np.arctan2(yy - cy, xx - cx) + 2.4))
    rim = np.exp(-((d - r) / (1.2 * s)) ** 2) * 60
    img = 7 + flange * (1 - inside) * (d < r + 30 * s) + liquid * inside + rim
    # Bolts.
    for k in range(36):
        a = k * 2 * math.pi / 36
        bx, by = cx + (r + 13 * s) * math.cos(a), cy + (r + 13 * s) * math.sin(a)
        bd = np.hypot(xx - bx, yy - by)
        img += np.clip(1 - bd / (2.6 * s), 0, 1) * 34
    grain = np.random.default_rng(3).normal(0, 1, (h, w)).astype(np.float32)
    grain = ndimage.gaussian_filter(grain, 0.7 * max(s, 0.6)) * (9 if s > 0.8 else 5)
    img = img + grain
    out = (img, inside, d)
    _BASE[(w, h)] = out
    return out


def fiducials(draw, s):
    """Fiducial crosses etched on the front and back windows (the back ones offset)."""
    for gx in range(-3, 4):
        for gy in range(-3, 4):
            x, y = CX + gx * 112 + 0, CY + gy * 112 - 6
            if math.hypot(x - CX, y - CY) > R - 30:
                continue
            for (ox, oy, c) in ((0, 0, 118), (3.5, 2.5, 62)):
                X, Y = (x + ox) * s, (y + oy) * s
                a = 6.5 * s
                draw.line([(X - a, Y), (X + a, Y)], fill=(c, c, c), width=max(1, round(s)))
                draw.line([(X, Y - a), (X, Y + a)], fill=(c, c, c), width=max(1, round(s)))


def render(cham, t, w, h, tau_fade=2.6, show_labels=True, counts=None):
    s = w / BASE_W
    img, inside, d = base_layer(w, h)
    canvas = img.copy()
    # Expansion flash: the chamber lights for an instant at every expansion.
    k = math.floor(t + 1e-6)
    since = t - k
    flash = math.exp(-since / 0.05)
    canvas += inside * flash * (13 + 11 * (1 - d / (R * s)))
    # Bubbles.
    if cham.bubbles:
        b = np.concatenate(cham.bubbles, 0)
        age = t - b[:, 2]
        keep = (age >= 0) & (age < tau_fade * 4.5)
        b, age = b[keep], age[keep]
        grow = np.sqrt(np.clip(age / 0.2, 0.15, 1))
        alpha = grow * np.exp(-age / tau_fade)
        small = np.zeros((h, w), np.float32)
        large = np.zeros((h, w), np.float32)
        x, y = b[:, 0] * s, b[:, 1] * s
        ok = (x > 1) & (x < w - 2) & (y > 1) & (y < h - 2)
        for buf, sel in ((small, ok & (b[:, 3] < 0.5)), (large, ok & (b[:, 3] >= 0.5))):
            xi, yi = np.floor(x[sel]).astype(int), np.floor(y[sel]).astype(int)
            fx, fy = x[sel] - xi, y[sel] - yi
            a = alpha[sel]
            np.add.at(buf, (yi, xi), a * (1 - fx) * (1 - fy))
            np.add.at(buf, (yi, xi + 1), a * fx * (1 - fy))
            np.add.at(buf, (yi + 1, xi), a * (1 - fx) * fy)
            np.add.at(buf, (yi + 1, xi + 1), a * fx * fy)
        sig_s, sig_l = 0.75 * s + 0.25, 1.25 * s + 0.3
        acc = (ndimage.gaussian_filter(small, sig_s) * (2 * math.pi * sig_s ** 2)
               + ndimage.gaussian_filter(large, sig_l) * (2 * math.pi * sig_l ** 2) * 1.2)
        halo = ndimage.gaussian_filter(acc, 3.2 * s + 0.5)
        lum = 238 * (1 - np.exp(-1.25 * acc)) + 60 * (1 - np.exp(-1.6 * halo))
        canvas += lum * inside * (1 - canvas / 300)
    image = S.to_image(np.repeat(canvas[..., None], 3, 2))
    draw = ImageDraw.Draw(image)
    fiducials(draw, s)
    # Film data box (decorative), with the expansion counter (the sample tick).
    f = S.font(max(9, round(15 * s)), TYPE)
    fs = S.font(max(8, round(12 * s)), TYPE)
    frame_no = 4100 + k
    draw.text((34 * s, 30 * s), "ISOTOP CHAMBER", font=f, fill=(170, 170, 170))
    draw.text((34 * s, 52 * s), f"FRAME {frame_no:06d}", font=f, fill=(170, 170, 170))
    draw.text((34 * s, 74 * s), "VIEW 1   B OUT OF PAGE", font=fs, fill=(110, 110, 110))
    draw.text((w - 34 * s, 30 * s), "EXPANSION 1.0 s", font=fs, fill=(110, 110, 110), anchor="ra")
    draw.text((w - 34 * s, 48 * s), "BUBBLES GROW 0.2 s", font=fs, fill=(110, 110, 110), anchor="ra")
    # Track annotations in typewriter type, placed greedily so no two overlap.
    placed = []

    def put(X, Y, text, anchor, font, force=False):
        box = draw.textbbox((X, Y), text, font=font, anchor=anchor)
        box = (box[0] - 4 * s, box[1] - 3 * s, box[2] + 4 * s, box[3] + 3 * s)
        if not force and any(not (box[2] < b[0] or box[0] > b[2] or box[3] < b[1] or box[1] > b[3])
                             for b in placed):
            return False
        placed.append(box)
        draw.text((X + 1, Y + 1), text, font=font, fill=(0, 0, 0), anchor=anchor)
        draw.text((X, Y), text, font=font, fill=(222, 222, 214), anchor=anchor)
        return True

    if show_labels:
        lf = S.font(max(11, round(14 * s)), TYPE)
        # The fork vertex (with the exec marker) and the exit burst get their own notes.
        if cham.fork_t + 0.6 < t < cham.fork_t + tau_fade * 1.6:
            vx, vy = cham.fork_vertex * s
            rr = 4.5 * s
            draw.ellipse([vx - rr, vy - rr, vx + rr, vy + rr], outline=(225, 225, 215), width=max(1, round(s)))
            for j, line in enumerate(("shell-170 fork", "find | grep")):
                put(vx - 10 * s, vy + (j - 0.5) * 16 * s, line, "rm", lf, force=True)
        for pr, pos, te in getattr(cham, "death_marks", []):
            if math.floor(te) + 1 <= t + 1e-6 and t - te < tau_fade * 1.6:
                lx, ly = CX + R + 26, pos[1] + 40
                draw.line([((pos[0] + 22) * s, (pos[1] + 10) * s), ((lx - 6) * s, ly * s)], fill=(200, 200, 194),
                          width=max(1, round(s)))
                for j, line in enumerate((f"{pr.name} exit", f"{pr.threads} delta rays, one per thread")):
                    put(lx * s, (ly + j * 17) * s, line, "lm", lf, force=True)
        for pr in cham.procs:
            if not pr.label or not hasattr(pr, "head"):
                continue
            last = pr.path[-1][0] if pr.path else -9
            # Only label tracks whose bubbles are still clearly visible.
            if t - last > tau_fade * 1.2 or t < pr.born + 1.0:
                continue
            # Annotate just outside the curl at a fixed angle from home, so labels stay put.
            ang = pr.label_angle
            r_now = pr.st[2] if hasattr(pr, "st") else pr.state(t)[2]
            lx = pr.home[0] + (r_now + 16) * math.cos(ang)
            ly = pr.home[1] + (r_now + 16) * math.sin(ang)
            put(lx * s, ly * s, pr.label, "lm" if math.cos(ang) >= -0.1 else "rm", lf)
    # Scanning-table sheet: the measured tracks of this frame (values from the simulation).
    trk = [pr for pr in cham.procs if pr.alive and pr.charged]
    trk.sort(key=lambda pr: -(pr.st if hasattr(pr, "st") else pr.state(t))[0])
    sf = S.font(max(9, round(13 * s)), TYPE)
    sx0, sy0 = 1236 * s, 132 * s
    lh = 19 * s
    draw.text((sx0, sy0), "SCAN SHEET", font=f, fill=(190, 190, 186))
    draw.text((sx0, sy0 + 1.4 * lh), "TRK PROCESS        q   p    beta  r", font=sf, fill=(120, 120, 116))
    for j, pr in enumerate(trk[:11]):
        p_, beta_, r_ = pr.st if hasattr(pr, "st") else pr.state(t)
        name = pr.name if len(pr.name) <= 14 else pr.name[:13] + "."
        row = f"{j + 1:>3} {name:<14} {'+' if pr.sign > 0 else '-'} {p_:4.2f} {beta_:5.2f} {r_:3.0f}"
        draw.text((sx0, sy0 + (2.6 + j) * lh), row, font=sf, fill=(205, 205, 198))
    yb = sy0 + (2.9 + min(len(trk), 11)) * lh
    for j, line in enumerate((f"neutral {192 - len(trk)}, no track", "p in cores of CPU", "r = p / (|q| B), px",
                              "beta = p / sqrt(p^2 + m^2)", "m ~ resident memory")):
        draw.text((sx0, yb + j * lh), line, font=sf, fill=(120, 120, 116))
    charged = sum(1 for pr in cham.procs if pr.alive and pr.charged)
    neutral = 192 - charged
    loc = 16240 + 900 * math.sin(k * 0.9)
    lines = [
        f"ISOTOP / CHAMBER / DEMO   192 processes | {charged} tracks | {neutral} neutral | "
        f"beam {int(round(loc / 5400))} = {loc:,.0f} LOC/s | B out of screen | events: proc connector",
        "track = busy process circling its home | radius = CPU | curl + user, - system | "
        "bubbles ~ 1/beta^2 (heavy = dense) | V = fork | curls = exit, one per thread",
    ]
    S.status(image, lines, size=max(9, round(13 * s)))
    return image


def build(events, t_end):
    cham = Sim(events)
    for pr in cham.procs:
        pr.last_pos = pr.home.copy() if pr.home is not None else np.zeros(2)
    cham.run(t_end)
    return cham


def main(which):
    if which in ("still", "both"):
        ev = {"spiral": (8.0, 2.4), "fork": 12.3, "exit": (13.45, 6)}
        cham = build(ev, 14.0)
        img = render(cham, 14.22, BASE_W, BASE_H, tau_fade=3.8)
        print(S.save_still(img, "chamber"))
    if which in ("anim", "both"):
        ev = {"spiral": (6.6, 2.4), "fork": 9.4, "exit": (13.3, 6)}
        cham = build(ev, 16.0)
        frames = []
        for i in range(120):
            t = 6.0 + i / 12.0
            # Rendering at t only uses bubbles born up to t, so one finished run serves all.
            sub = Snapshot(cham, t)
            frames.append(render(sub, t, 960, 540, tau_fade=2.6))
        print(S.save_frames(frames, "chamber", fps=12, colors=48))
        print(gifpack.pack("chamber", colors=48))


class Snapshot:
    """A view of the finished run as it stood at time t (bubbles and track heads)."""
    def __init__(self, cham, t):
        self.bubbles = [b[b[:, 2] <= t + 1e-6] for b in cham.bubbles]
        self.fork_vertex, self.fork_t = cham.fork_vertex, cham.fork_t
        self.death_marks = [d for d in getattr(cham, "death_marks", []) if d[2] <= t]
        self.procs = []
        for pr in cham.procs:
            v = _View(pr, t)
            self.procs.append(v)


class _View:
    def __init__(self, pr, t):
        self.label, self.label_angle, self.born, self.died = pr.label, pr.label_angle, pr.born, pr.died
        self.name, self.sign, self.mem, self.threads = pr.name, pr.sign, pr.mem, pr.threads
        self.st = pr.state(t)
        self.home = pr.home
        self.label_pick = getattr(pr, 'label_pick', 0.0)
        k = math.floor(t + 1e-6)
        seg = [q for q in pr.path if q[0] <= k]
        self.path = seg
        self.alive = t >= pr.born and (pr.died is None or t < pr.died)
        p = pr.cpu(t) if self.alive else 0
        self.charged = self.alive and p >= NEUTRAL_CPU
        if len(seg) >= 2:
            self.head = np.array(seg[-1][1:3])


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "both")
