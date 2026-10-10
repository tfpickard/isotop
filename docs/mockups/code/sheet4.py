import sys, os
from PIL import Image
OUT = "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/out"
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
