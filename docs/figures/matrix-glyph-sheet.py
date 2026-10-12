"""The Matrix view's bitmap font, decoded from src/glyphs.rs.

Run from the repository root: python3 docs/figures/matrix-glyph-sheet.py
Writes docs/media/figures/matrix-glyph-sheet.svg. Standard library only.

glyphs.rs stores each 7 x 14 glyph of printable ASCII (the X11 misc-fixed 7x14 font) as a u128:
row r, column c is bit r * 7 + c, column 0 at the left. This script reads the hexadecimal
constants out of the Rust source, so the sheet is the font the view draws, not a copy of it.
Left: all 95 glyphs. Right: the letter 'g' enlarged with every pixel's bit index, to show the
packing (98 of the 128 bits are used, so the top 30 are always zero).
"""

import os
import re
import sys

sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from svgkit_w6 import AQUA, BLUE, MUTED, ORANGE, TEXT, Svg, heading

WIDTH, HEIGHT = 7, 14


def load():
    source = open(os.path.join("src", "glyphs.rs"), encoding="utf-8").read()
    body = source[source.index("pub const GLYPHS"):source.index("/// The glyph for a character")]
    values = [int(m, 16) for m in re.findall(r"0x([0-9a-fA-F]+),", body)]
    assert len(values) == 95, len(values)
    return values


def main():
    glyphs = load()
    columns, cell = 19, 4
    gap_x, gap_y = 6, 6
    pitch_x, pitch_y = WIDTH * cell + gap_x, HEIGHT * cell + gap_y
    rows = -(-95 // columns)
    sheet_w, sheet_h = columns * pitch_x, rows * pitch_y
    svg = Svg(sheet_w + 24 + 420, max(sheet_h + 90, 480), "The Matrix view's 7 by 14 bitmap font",
              "All 95 printable ASCII glyphs of the 7x14 font in src/glyphs.rs, and the letter g "
              "enlarged with its bit numbering.")
    heading(svg, "7 x 14 bitmap font, decoded from src/glyphs.rs",
            "95 glyphs, space (0x20) to tilde (0x7e); bit r * 7 + c is row r, column c")
    ox, oy = 24, 70
    for index, bits in enumerate(glyphs):
        gx = ox + (index % columns) * pitch_x
        gy = oy + (index // columns) * pitch_y
        svg.rect(gx - 1, gy - 1, WIDTH * cell + 2, HEIGHT * cell + 2, "#161b22")
        for r in range(HEIGHT):
            for c in range(WIDTH):
                if bits >> (r * WIDTH + c) & 1:
                    svg.rect(gx + c * cell, gy + r * cell, cell, cell, "#4cd96a")
    # the enlarged glyph
    letter = glyphs[ord("g") - 0x20]
    big = 20
    bx, by = ox + sheet_w + 40, 86
    svg.text(bx, by - 14, "'g' = 0x%025x" % letter, 12, "start", TEXT, mono=True)
    for r in range(HEIGHT):
        for c in range(WIDTH):
            on = letter >> (r * WIDTH + c) & 1
            svg.rect(bx + c * big, by + r * big, big - 1, big - 1, "#4cd96a" if on else "#161b22")
            svg.text(bx + c * big + big / 2, by + r * big + big / 2 + 3.2, str(r * WIDTH + c), 7.2,
                     "middle", "#0e1116" if on else "#6e7681")
    svg.text(bx + WIDTH * big + 14, by + 18, "column 0 at the left,", 11, "start", MUTED)
    svg.text(bx + WIDTH * big + 14, by + 34, "row 0 at the top", 11, "start", MUTED)
    svg.text(bx + WIDTH * big + 14, by + 66, "rows 0-1 and 12-13", 11, "start", MUTED)
    svg.text(bx + WIDTH * big + 14, by + 82, "hold ascenders and", 11, "start", MUTED)
    svg.text(bx + WIDTH * big + 14, by + 98, "descenders: the font", 11, "start", MUTED)
    svg.text(bx + WIDTH * big + 14, by + 114, "includes the spacing", 11, "start", MUTED)
    print(svg.save("matrix-glyph-sheet"))
    used = bin(max(glyphs)).count("1")
    print("highest set bit", max(g.bit_length() for g in glyphs) - 1, "ink pixels in 'g':", bin(letter).count("1"))


if __name__ == "__main__":
    main()
