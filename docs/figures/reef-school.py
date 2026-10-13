"""Reef: the sunflower seating of a colony and the loops of a school, from src/reef.rs.

Writes docs/media/figures/reef-school.svg. Run from the repository root:

    python3 docs/figures/reef-school.py

Left: the seats of a colony. A process's seat `k` (stable for life) sits on a Vogel spiral at
distance 1.6 sqrt(k + 0.5) and angle k * 2.399963 rad (the golden angle, 137.5 degrees); a colony
of `span` seats reserves a disc of radius 1.6 sqrt(span) + 2, and a polyp's stalk is
1.5 + 5 (1 - distance / room) tall. Right: the steering rule of `Reef::school`, run for 40 s on a
school of ten fish, once at 3 % CPU (speed limit 0.8 + 5 c / (c + 40) = 1.2) and once at 100 %
(4.4). Each step, with the school's state taken before any fish moves,

    v += dt * [ 0.4 (target - x)  +  sum over mates with 1e-4 < d^2 < 6 of 1.5 (x - x_j) / d^2
                + 0.5 (mean(v) - v) ]
    |v| clamped to [0.3, 1] * speed,   x += v dt,   dt <= 0.1 s.

The centre of the school obeys X'' = 0.4 (target - X) exactly (the other two terms cancel in the
sum), an undamped oscillator of angular frequency sqrt(0.4) = 0.63 rad/s (period 9.9 s) about a
target that itself circles slowly; the speed limit only clips it. The school swings back and forth
through the target instead of settling on it. The replica below is this rule, not the renderer.
"""

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import svgplot_views_cores_reef as S  # noqa: E402

GOLDEN = 2.399963
DEPTH = 14.0


def cpu_speed(cpu):
    return 0.8 + 5.0 * cpu / (cpu + 40.0)


def simulate(cpu, fish=10, seconds=40.0, dt=0.1):
    phase = 0.7
    room = 6.0
    speed = cpu_speed(cpu)
    state = []
    for i in range(fish):
        a = (i * 0.61803398875 % 1.0) * 2 * math.pi
        r = 0.8 + 0.35 * i
        pos = [r * math.cos(a), r * math.sin(a), 5.0 + 0.2 * ((i % 3) - 1)]
        state.append([pos, [math.cos(phase), math.sin(phase), 0.0]])
    tracks = [[] for _ in range(fish)]
    centre = []
    targets = []
    steps = int(seconds / dt)
    for step in range(steps + 1):
        t = step * dt
        target = [room * 0.4 * math.cos(0.05 * t + phase), room * 0.4 * math.sin(0.05 * t + phase),
                  5.0 + 2.5 * math.sin(0.07 * t + phase)]
        targets.append((target[0], target[1]))
        old = [(list(p), list(v)) for p, v in state]
        mean = [sum(v[k] for _, v in old) / fish for k in range(3)]
        for index, (pos, vel) in enumerate(state):
            push = [(target[k] - pos[k]) * 0.4 for k in range(3)]
            for other, (opos, _) in enumerate(old):
                if other == index:
                    continue
                away = [pos[k] - opos[k] for k in range(3)]
                d2 = sum(c * c for c in away)
                if 1e-4 < d2 < 6.0:
                    for k in range(3):
                        push[k] += away[k] / d2 * 1.5
            old_vel = old[index][1]
            for k in range(3):
                push[k] += (mean[k] - old_vel[k]) * 0.5
            for k in range(3):
                vel[k] += push[k] * dt
            length = max(math.sqrt(sum(c * c for c in vel)), 1e-3)
            clamped = min(max(length, speed * 0.3), speed)
            for k in range(3):
                vel[k] = vel[k] / length * clamped
                pos[k] += vel[k] * dt
            pos[2] = min(max(pos[2], 1.0), DEPTH - 0.5)
            tracks[index].append((pos[0], pos[1]))
        centre.append((sum(p[0] for p, _ in state) / fish, sum(p[1] for p, _ in state) / fish))
    return tracks, centre, speed, targets


def spread(tracks, centre, tail=200):
    total, count = 0.0, 0
    for track in tracks:
        for (x, y), (cx, cy) in list(zip(track, centre))[-tail:]:
            total += math.hypot(x - cx, y - cy)
            count += 1
    return total / count


def main():
    svg = S.Svg(1060, 494, "Reef: colony seating and school loops",
                "Golden-angle seats of a coral colony, and the paths of a school of ten fish at "
                "low and high CPU (a replica of the steering rule).")
    # Left: phyllotaxis.
    spans = 40
    room = 1.6 * math.sqrt(spans) + 2
    cx, cy, scale = 195.0, 250.0, 13.0
    svg.text(30, 34, "Seats: r = 1.6 sqrt(k + 0.5), angle = k * 137.5 deg", 13, weight="600")
    svg.circle(cx, cy, room * scale, "#12263a", S.AXIS, 1.2)
    for k in range(spans):
        d = 1.6 * math.sqrt(k + 0.5)
        a = k * GOLDEN
        tall = 1.5 + 5.0 * max(1 - d / room, 0.0)
        svg.circle(cx + d * scale * math.cos(a), cy - d * scale * math.sin(a), 2.2 + tall * 0.9,
                   S.mix(S.TEAL, S.GOLD, k / spans), "#0e1116", 0.6, opacity=0.9)
    svg.text(cx, cy + room * scale + 22, "colony of 40 seats reserves radius %.1f" % room, 11, "middle",
             S.MUTED)
    svg.text(cx, cy + room * scale + 38, "dot size = stalk height 1.5 + 5 (1 - d / room): a dome", 11,
             "middle", S.MUTED)

    # Right: two schools.
    for index, cpu in enumerate((3.0, 100.0)):
        tracks, centre, speed, targets = simulate(cpu)
        left = 400 + index * 330
        ax = S.Axes(svg, left, 70, 300, 300, (-9, 9), (-9, 9))
        ax.frame(xticks=[-8, -4, 0, 4, 8], yticks=[-8, -4, 0, 4, 8],
                 xlabel="x (world units)", ylabel="y" if index == 0 else "")
        svg.text(left, 56, "%d %% CPU: speed limit %.1f" % (cpu, speed), 13, weight="600")
        for track in tracks:
            svg.polyline([ax.pt(x, y) for x, y in track[::2]], S.CYAN, 0.9, opacity=0.35)
        svg.polyline([ax.pt(x, y) for x, y in centre], S.GOLD, 1.8)
        for track in tracks:
            x, y = track[-1]
            svg.circle(ax.px(x), ax.py(y), 3.2, S.TEXT, "#0e1116", 0.8)
        excursion = max(math.hypot(c[0] - t[0], c[1] - t[1]) for c, t in zip(centre, targets))
        svg.text(left, 426, "largest swing of the centre past the target: %.1f" % excursion, 10.5,
                 fill=S.MUTED)
        print("cpu", cpu, "speed", round(speed, 2), "spread", round(spread(tracks, centre), 2),
              "max centre excursion", round(excursion, 2),
              "speed / omega", round(speed / math.sqrt(0.4), 2))
    svg.text(400, 452, "gold = the school's centre (a driven undamped oscillator), blue = the fish's "
             "paths over 40 s, dots = positions at 40 s", 11, fill=S.MUTED)
    svg.text(400, 470, "Equal and opposite separation and alignment terms leave only the pull "
             "toward the circling target to move the centre.", 11, fill=S.MUTED)
    print(svg.save("reef-school"))


if __name__ == "__main__":
    main()
