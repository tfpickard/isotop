import sys, os
from PIL import Image
# Rendered stills, frames and GIFs go to $ISOTOP_MOCKUP_OUT, or docs/mockups/out by default.
OUT = os.environ.get("ISOTOP_MOCKUP_OUT",
                     os.path.join(os.path.dirname(os.path.abspath(__file__)), os.pardir, "out"))
os.makedirs(OUT, exist_ok=True)
name = sys.argv[1]
d = os.path.join(OUT, f"{name}-frames")
fs = sorted(os.listdir(d))
idx = [int(a) for a in sys.argv[2:]] or [0, len(fs) // 3, 2 * len(fs) // 3, len(fs) - 1]
ims = [Image.open(os.path.join(d, fs[i])) for i in idx]
w, h = ims[0].size
sheet = Image.new("RGB", (w * 2, h * 2))
for k, im in enumerate(ims):
    sheet.paste(im, ((k % 2) * w, (k // 2) * h))
sheet.save(os.path.join(OUT, f"{name}-sheet.png"))
print(idx)
