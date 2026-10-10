"""Render each Gray-Scott preset on its own small grid to see which morphologies are distinct."""
import sys
import numpy as np
from PIL import Image

# Sims' 9-point kernel approximates 0.3 * laplacian, so these match D = 0.2097 / 0.105 on a
# standard 5-point stencil (the units the classic presets assume).
DU, DV = 0.2097 / 0.3, 0.105 / 0.3
PRESETS = [
    ("solitons", 0.030, 0.062),
    ("mitosis", 0.0367, 0.0649),
    ("fingerprint", 0.037, 0.060),
    ("mazes", 0.029, 0.057),
    ("holes", 0.039, 0.058),
    ("chaos", 0.026, 0.051),
    ("moving spots", 0.014, 0.054),
    ("waves", 0.014, 0.045),
    ("u-skate", 0.062, 0.06093),
    ("worms", 0.078, 0.061),
    ("coral", 0.0545, 0.062),
    ("spots+loops", 0.018, 0.051),
]


def laplacian(a):
    return (
        0.2 * (np.roll(a, 1, 0) + np.roll(a, -1, 0) + np.roll(a, 1, 1) + np.roll(a, -1, 1))
        + 0.05 * (np.roll(np.roll(a, 1, 0), 1, 1) + np.roll(np.roll(a, 1, 0), -1, 1)
                  + np.roll(np.roll(a, -1, 0), 1, 1) + np.roll(np.roll(a, -1, 0), -1, 1))
        - a
    )


def run(f, k, size=128, steps=6000, seed=1):
    rng = np.random.default_rng(seed)
    u = np.ones((size, size))
    v = np.zeros((size, size))
    for _ in range(12):
        x, y = rng.integers(10, size - 10, 2)
        u[y - 3:y + 3, x - 3:x + 3] = 0.5
        v[y - 3:y + 3, x - 3:x + 3] = 0.25
    v += 0.01 * rng.random((size, size))
    # The grid-unit diffusion here is ~4.8x smaller than Sims' (1.0, 0.5), so scale by a
    # diffusion-equivalent factor: these presets assume D = 0.2097/0.105 with dt = 1.
    for _ in range(steps):
        uvv = u * v * v
        u += DU * laplacian(u) - uvv + f * (1 - u)
        v += DV * laplacian(v) + uvv - (f + k) * v
    return v


def main(out):
    tiles = []
    for name, f, k in PRESETS:
        v = run(f, k)
        g = np.clip(v / max(v.max(), 1e-6), 0, 1)
        tiles.append((name, (g * 255).astype(np.uint8)))
    cols = 4
    rows = (len(tiles) + cols - 1) // cols
    sheet = Image.new("L", (cols * 132, rows * 132), 0)
    for i, (name, t) in enumerate(tiles):
        sheet.paste(Image.fromarray(t), ((i % cols) * 132, (i // cols) * 132))
        print(i, name)
    sheet.save(out)


if __name__ == "__main__":
    main(sys.argv[1])
