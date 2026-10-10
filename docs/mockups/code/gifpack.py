"""Re-encode a view's saved PNG frames as a compact GIF with ffmpeg: one global palette and
transparent pixels wherever a frame repeats the previous one (ffmpeg's transdiff)."""
import os
import subprocess

OUT = "/tmp/claude-0/-home-claude/5721c45e-59eb-54a5-91b7-cfe434980b29/scratchpad/out"


def pack(name, fps=12, width=720, colors=128, dither="none", stats="full"):
    frames = os.path.join(OUT, f"{name}-frames", "%04d.png")
    out = os.path.join(OUT, f"{name}.gif")
    tmp = out + ".tmp.gif"
    filt = (f"fps={fps},scale={width}:-1:flags=lanczos,split[a][b];"
            f"[a]palettegen=max_colors={colors}:stats_mode={stats}[p];"
            f"[b][p]paletteuse=dither={dither}:diff_mode=rectangle")
    subprocess.run(["ffmpeg", "-v", "error", "-y", "-framerate", str(fps), "-i", frames,
                    "-filter_complex", filt, "-loop", "0", tmp], check=True)
    os.replace(tmp, out)
    return out, os.path.getsize(out)
