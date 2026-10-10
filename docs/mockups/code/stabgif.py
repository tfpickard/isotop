"""Pack a view's saved frames into a small GIF: resize to <= 720 wide, then hold pixels that
changed by less than `thresh` (per channel, max) from the previously shown frame, so the GIF's
frame differencing can skip them. One global palette via ffmpeg."""
import os
import shutil
import subprocess
import sys

import numpy as np
from PIL import Image

# Rendered stills, frames and GIFs go to $ISOTOP_MOCKUP_OUT, or docs/mockups/out by default.
OUT = os.environ.get("ISOTOP_MOCKUP_OUT",
                     os.path.join(os.path.dirname(os.path.abspath(__file__)), os.pardir, "out"))
os.makedirs(OUT, exist_ok=True)


def pack(name, fps=12, width=720, colors=128, thresh=10, stats="full"):
    src = os.path.join(OUT, f"{name}-frames")
    tmp = os.path.join(OUT, f".{name}-stab")
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp)
    prev = None
    for i, fn in enumerate(sorted(os.listdir(src))):
        im = Image.open(os.path.join(src, fn)).convert("RGB")
        if im.width > width:
            im = im.resize((width, round(im.height * width / im.width)), Image.LANCZOS)
        a = np.asarray(im, np.int16)
        if prev is not None:
            same = np.abs(a - prev).max(axis=2) < thresh
            a = np.where(same[..., None], prev, a)
        prev = a
        Image.fromarray(a.astype(np.uint8)).save(os.path.join(tmp, f"{i:04d}.png"))
    out = os.path.join(OUT, f"{name}.gif")
    filt = (f"split[a][b];[a]palettegen=max_colors={colors}:stats_mode={stats}[p];"
            f"[b][p]paletteuse=dither=none:diff_mode=rectangle")
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-framerate", str(fps), "-i", os.path.join(tmp, "%04d.png"),
                    "-filter_complex", filt, "-loop", "0", out + ".tmp.gif"], check=True)
    os.replace(out + ".tmp.gif", out)
    shutil.rmtree(tmp, ignore_errors=True)
    return out, os.path.getsize(out)


if __name__ == "__main__":
    kw = {}
    for a in sys.argv[2:]:
        k, v = a.split("=")
        kw[k] = int(v)
    print(pack(sys.argv[1], **kw))
