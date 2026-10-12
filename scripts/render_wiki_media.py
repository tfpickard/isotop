#!/usr/bin/env python3
"""Regenerates the wiki's rendered media in docs/media/renders/.

Run from the repository root after `cargo build --release`:

    python3 scripts/render_wiki_media.py              # everything
    python3 scripts/render_wiki_media.py --only reef  # one view: still, loop and filmstrip
    python3 scripts/render_wiki_media.py --only stills       # also: contact-sheet, loops, filmstrips
    python3 scripts/render_wiki_media.py --check-tools

Standard library only. It calls the isotop binary (CPU rasterizer, demo workload, so every
output is repeatable), ImageMagick (`convert`, `montage`) and ffmpeg (libwebp_anim).

What it writes, all WebP:

    <view>.webp            a 1600 px still of the view at STILLS[view] seconds of demo time
    <view>-loop.webp       a LOOPS[view] animation: consecutive frames of one run (see below)
    filmstrip-<view>.webp  FILMSTRIPS[view]: four panels at increasing demo times
    contact-sheet.webp     every still, labelled with the view's name

Loop frames. A headless run draws one frame at --time. `--benchmark N --output f.png` draws N
frames at --time + i / --fps and writes the last one, so frame k of a loop is the run with
--benchmark k+1: each frame is a point on one continuous trajectory of the view's simulation,
and the demo snapshot stays fixed (only animation time moves). Successive --time values would
instead re-run every warm-up and re-lay the scene, which is not an animation.

Each loop and still is encoded at the highest quality whose file fits its size budget; the
script fails if even the lowest quality does not fit, so a table change that bloats a file is
caught here.
"""

import argparse
import concurrent.futures
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BINARY = REPO / "target" / "release" / "isotop"
OUTPUT = REPO / "docs" / "media" / "renders"

# ---------------------------------------------------------------------------------------------
# The choices. Demo seconds (`--time`) at which each view looks best and is mature.
#
# Why these times:
#   * ptolemy needs 30 samples before it has planets and 60 for a full window, and its
#     reference renders use 300; tesseract replays one minute of worldlines (full at 60) and
#     turns its 4D camera a full turn every 120 s, so 120 is the canonical orientation;
#   * strata and necropolis replay their window from before time 0, so they are full at any
#     time (necropolis replays ten minutes of deaths);
#   * the necropolis still sits at demo day 15.97, a real full moon (its moon is computed
#     from the demo wall clock, DEMO_EPOCH + time), at a time where the demo's memory-pressure
#     wave sin(0.13 t) is at its trough, so the fog is at its base density.
# ---------------------------------------------------------------------------------------------
STILLS = {
    "city": 60,
    "orbit": 60,
    "ripple": 12,
    "flow": 60,
    "cores": 60,
    "cells": 150,
    "strata": 60,
    "globe": 12,
    "reef": 60,
    "matrix": 60,
    "coop": 60,
    "ptolemy": 300,
    "tesseract": 120,
    "necropolis": 1383062,
}

# The contact sheet's order is the registry's (Tab) order.
ORDER = list(STILLS)

# view: (start time in demo seconds, loop seconds, frames per second). Width is LOOP_WIDTH.
# necropolis has no loop: a headless frame of it is static (only its fog wisps drift, by well
# under one grey level per second), and successive --time values re-lay its graves.
LOOPS = {
    "ripple": (12, 5, 10),
    "flow": (60, 5, 10),
    "cores": (60, 5, 10),
    "coop": (60, 5, 10),
    "reef": (60, 5, 10),
    "orbit": (60, 5, 10),
    "ptolemy": (300, 5, 10),
    "tesseract": (120, 5, 10),
}

# view: [(demo seconds, label or None for "t = N s"), ...], four panels.
FILMSTRIPS = {
    # The minute of history fills as the run gets older; strata replays its window from before
    # time 0, so these show the window sliding, not growing.
    "strata": [(10, None), (30, None), (60, None), (120, None)],
    # Planets need 30 samples; the window is full at 60.
    "ptolemy": [(15, None), (30, None), (45, None), (60, None)],
    # One minute of worldlines is replayed from time 0, so the worldlines lengthen until 60 s.
    "tesseract": [(10, None), (25, None), (40, None), (60, None)],
    # The graves are always ten minutes of deaths; what advances is the moon, computed from
    # the demo's wall clock: a thin crescent (the demo epoch is 1.2 days before new moon), then
    # first quarter, full and last quarter, each at a time where the memory-pressure wave is at
    # its trough (so the fog does not change between panels).
    "necropolis": [
        (0, "t = 0 d, thin crescent"),
        (742371, "t = 8.6 d, first quarter"),
        (1383062, "t = 16.0 d, full moon"),
        (2018098, "t = 23.4 d, last quarter"),
    ],
}

STILL_WIDTH = 1600
LOOP_WIDTH = 960
CONTACT_WIDTH = 2400
CONTACT_COLUMNS = 4
FILMSTRIP_WIDTH = 2400

# Size budgets in bytes (decimal kilobytes, so they hold under either reading of "KB").
STILL_BUDGET = 450_000
LOOP_BUDGET = 1_600_000
CONTACT_BUDGET = 900_000
FILMSTRIP_BUDGET = 700_000

GROUPS = ["stills", "contact-sheet", "loops", "filmstrips"]

STILL_QUALITIES = (95, 90, 85, 80, 75, 70, 65, 60, 55, 50, 45, 40)
LOOP_QUALITIES = (80, 72, 64, 56, 48, 40, 32)
SHEET_QUALITIES = (95, 90, 85, 80, 75, 70, 65, 60, 55, 50, 45, 40)

BACKGROUND = "#0e1116"
FOREGROUND = "#c9d1d9"


def tool(*names):
    """The first of `names` found on PATH, as an argv prefix list, or None."""
    for name in names:
        if shutil.which(name.split()[0]):
            return name.split()
    return None


CONVERT = tool("convert", "magick")
MONTAGE = tool("montage", "magick montage")


def run(argv, **kwargs):
    """Runs a command, failing loudly with its output."""
    result = subprocess.run(argv, capture_output=True, text=True, **kwargs)
    if result.returncode != 0:
        sys.exit(
            f"command failed ({result.returncode}): {' '.join(map(str, argv))}\n"
            f"{result.stdout}{result.stderr}"
        )
    return result


def check_tools():
    """Reports each required tool and returns whether all are usable."""
    ok = True

    def report(name, good, detail):
        nonlocal ok
        ok = ok and good
        print(f"{'ok     ' if good else 'MISSING'} {name}: {detail}")

    if BINARY.is_file() and os.access(BINARY, os.X_OK):
        version = subprocess.run([BINARY, "--version"], capture_output=True, text=True)
        report("isotop", version.returncode == 0, f"{BINARY} ({version.stdout.strip()})")
    else:
        report("isotop", False, f"{BINARY} not built (cargo build --release)")
    if CONVERT:
        formats = subprocess.run(
            CONVERT + ["-list", "format"], capture_output=True, text=True
        ).stdout
        webp = any(line.split()[:1] == ["WEBP*"] or line.split()[:1] == ["WEBP"]
                   for line in formats.splitlines())
        report("ImageMagick convert", webp, "WebP read/write" if webp else "no WebP delegate")
    else:
        report("ImageMagick convert", False, "not on PATH")
    report("ImageMagick montage", MONTAGE is not None, "found" if MONTAGE else "not on PATH")
    if shutil.which("ffmpeg"):
        encoders = subprocess.run(
            ["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True
        ).stdout
        report("ffmpeg", "libwebp_anim" in encoders, "libwebp_anim encoder"
               if "libwebp_anim" in encoders else "no libwebp_anim encoder")
    else:
        report("ffmpeg", False, "not on PATH")
    return ok


def isotop(args):
    run([BINARY, "--demo", "--renderer", "cpu"] + [str(arg) for arg in args])


def encode_still(source, target, budget, qualities, extra=()):
    """Writes `source` as WebP at the highest quality that fits `budget`."""
    for quality in qualities:
        run(
            CONVERT
            + [str(source), *extra, "-define", "webp:method=6", "-quality", str(quality),
               str(target)]
        )
        if target.stat().st_size <= budget:
            return quality
    sys.exit(f"{target.name}: {target.stat().st_size} bytes exceeds {budget} at the lowest quality")


def time_label(seconds):
    return f"t = {seconds / 86400:.1f} d" if seconds >= 86400 else f"t = {seconds:g} s"


class Job:
    """One run of the script: where scratch files go and what has been rendered so far."""

    def __init__(self, work, output, jobs):
        self.work = Path(work)
        self.output = Path(output)
        self.jobs = jobs
        self.pngs = {}

    def png(self, view, seconds, width):
        """A headless PNG of `view` at `seconds`, rendered once per run."""
        key = (view, seconds, width)
        if key not in self.pngs:
            path = self.work / f"{view}_{seconds}_{width}.png"
            isotop(["--view", view, "--width", width, "--time", seconds, "--output", path])
            self.pngs[key] = path
        return self.pngs[key]

    def still(self, view):
        target = self.output / f"{view}.webp"
        quality = encode_still(
            self.png(view, STILLS[view], STILL_WIDTH), target, STILL_BUDGET, STILL_QUALITIES
        )
        report(target, f"t={STILLS[view]} q{quality}")

    def loop(self, view):
        start, seconds, fps = LOOPS[view]
        count = seconds * fps
        folder = self.work / f"loop_{view}"
        folder.mkdir(exist_ok=True)

        def frame(index):
            isotop(["--view", view, "--width", LOOP_WIDTH, "--time", start, "--fps", fps,
                    "--benchmark", index + 1, "--output", folder / f"f_{index:03d}.png"])

        with concurrent.futures.ThreadPoolExecutor(self.jobs) as pool:
            list(pool.map(frame, range(count)))
        target = self.output / f"{view}-loop.webp"
        for quality in LOOP_QUALITIES:
            run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(fps), "-i",
                 str(folder / "f_%03d.png"), "-c:v", "libwebp_anim", "-lossless", "0",
                 "-quality", str(quality), "-compression_level", "6", "-loop", "0",
                 str(target)])
            if target.stat().st_size <= LOOP_BUDGET:
                report(target, f"t={start}+{seconds}s {fps}fps {count} frames q{quality}")
                return
        sys.exit(f"{target.name}: over {LOOP_BUDGET} bytes at the lowest quality")

    def montage(self, panels, columns, width, target, budget, qualities, pointsize):
        """Labelled tiles of (png path, label) in a grid `width` wide, as one WebP."""
        gutter = 4
        tile = width // columns - 2 * gutter
        sheet = self.work / (target.stem + ".png")
        argv = MONTAGE + ["-background", BACKGROUND, "-fill", FOREGROUND,
                          "-pointsize", str(pointsize)]
        if font_available("DejaVu-Sans"):
            argv += ["-font", "DejaVu-Sans"]
        argv += ["-tile", f"{columns}x", "-geometry", f"{tile}x+{gutter}+{gutter}"]
        for path, label in panels:
            argv += ["-label", label, str(path)]
        run(argv + [str(sheet)])
        quality = encode_still(sheet, target, budget, qualities)
        report(target, f"q{quality}")

    def filmstrip(self, view):
        panels = [
            (self.png(view, seconds, STILL_WIDTH), label or time_label(seconds))
            for seconds, label in FILMSTRIPS[view]
        ]
        self.montage(panels, len(panels), FILMSTRIP_WIDTH,
                     self.output / f"filmstrip-{view}.webp", FILMSTRIP_BUDGET, STILL_QUALITIES,
                     26)

    def contact_sheet(self):
        panels = [(self.png(view, STILLS[view], STILL_WIDTH), view) for view in ORDER]
        self.montage(panels, CONTACT_COLUMNS, CONTACT_WIDTH, self.output / "contact-sheet.webp",
                     CONTACT_BUDGET, SHEET_QUALITIES, 26)


def font_available(name):
    fonts = subprocess.run(CONVERT + ["-list", "font"], capture_output=True, text=True).stdout
    return f"Font: {name}\n" in fonts or f"Font: {name}" in fonts.split("\n")


def report(path, detail):
    print(f"{path.name:28} {path.stat().st_size / 1000:8.1f} KB  {detail}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--only", metavar="NAME",
                        help="one view (its still, loop and filmstrip), or a group: "
                             + ", ".join(GROUPS))
    parser.add_argument("--check-tools", action="store_true",
                        help="check isotop, ImageMagick and ffmpeg and exit")
    parser.add_argument("--output", type=Path, default=OUTPUT, help="output directory")
    parser.add_argument("--jobs", type=int, default=os.cpu_count() or 1,
                        help="parallel isotop processes for loop frames")
    args = parser.parse_args()
    if args.check_tools:
        sys.exit(0 if check_tools() else 1)
    if not check_tools():
        sys.exit("missing tools (see above)")
    if args.only and args.only not in STILLS and args.only not in GROUPS:
        sys.exit(f"--only: unknown name {args.only!r}; one of {', '.join(ORDER + GROUPS)}")
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="isotop-wiki-media-") as work:
        job = Job(work, args.output, args.jobs)
        if args.only in STILLS:
            job.still(args.only)
            if args.only in LOOPS:
                job.loop(args.only)
            if args.only in FILMSTRIPS:
                job.filmstrip(args.only)
            return
        # Everything, in the order of how much the wiki needs it.
        everything = args.only is None
        if everything or args.only == "stills":
            for view in ORDER:
                job.still(view)
        if everything or args.only == "contact-sheet":
            job.contact_sheet()
        if everything or args.only == "loops":
            for view in LOOPS:
                job.loop(view)
        if everything or args.only == "filmstrips":
            for view in FILMSTRIPS:
                job.filmstrip(view)


if __name__ == "__main__":
    main()
