#!/usr/bin/env python3
"""Compare two PNG renders and report how far apart they are.

Standard library only. Decodes non-interlaced 8-bit RGB and RGBA PNGs (all five scanline
filters) and ignores alpha, because isotop writes RGB.

Prints one line:
  mean M | share S | max X | size WxH
where M is the per-channel mean absolute difference (0..255), S is the percent of pixels where
any channel differs by more than --levels, and X is the largest channel difference.

Exit 0 when within limits, 1 when the sizes differ or a limit is exceeded, 2 when a file cannot
be decoded. The default limits are the tolerance recorded in docs/VIEW_CONVENTIONS.md.
"""

import argparse
import struct
import sys
import zlib

SIGNATURE = b"\x89PNG\r\n\x1a\n"
DEFAULT_MAX_MEAN = 0.25
DEFAULT_MAX_SHARE = 0.25
DEFAULT_LEVELS = 2


class PngError(Exception):
    pass


def paeth(left, up, up_left):
    estimate = left + up - up_left
    distance_left = abs(estimate - left)
    distance_up = abs(estimate - up)
    distance_up_left = abs(estimate - up_left)
    if distance_left <= distance_up and distance_left <= distance_up_left:
        return left
    if distance_up <= distance_up_left:
        return up
    return up_left


def decode(data):
    """Returns (width, height, rows) where each row is a bytearray of RGB triples."""
    if data[:8] != SIGNATURE:
        raise PngError("not a PNG file")
    position = 8
    header = None
    compressed = []
    while position + 8 <= len(data):
        length, kind = struct.unpack(">I4s", data[position : position + 8])
        body = data[position + 8 : position + 8 + length]
        if len(body) != length:
            raise PngError("truncated chunk")
        position += 12 + length
        if kind == b"IHDR":
            header = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            compressed.append(body)
        elif kind == b"IEND":
            break
    if header is None:
        raise PngError("missing IHDR")
    width, height, depth, color_type, _compression, _filter, interlace = header
    if interlace != 0:
        raise PngError("interlaced PNGs are not supported")
    if depth != 8 or color_type not in (2, 6):
        raise PngError(f"only 8-bit RGB and RGBA are supported (depth {depth}, color type {color_type})")
    channels = 3 if color_type == 2 else 4
    stride = width * channels
    try:
        raw = zlib.decompress(b"".join(compressed))
    except zlib.error as error:
        raise PngError(f"bad image data: {error}")
    if len(raw) != (stride + 1) * height:
        raise PngError("image data has the wrong length")

    rows = []
    previous = bytearray(stride)
    for y in range(height):
        start = y * (stride + 1)
        filter_type = raw[start]
        line = bytearray(raw[start + 1 : start + 1 + stride])
        if filter_type == 0:
            pass
        elif filter_type == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 255
        elif filter_type == 2:
            for i in range(stride):
                line[i] = (line[i] + previous[i]) & 255
        elif filter_type == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + previous[i]) >> 1)) & 255
        elif filter_type == 4:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                up_left = previous[i - channels] if i >= channels else 0
                line[i] = (line[i] + paeth(left, previous[i], up_left)) & 255
        else:
            raise PngError(f"unknown filter type {filter_type}")
        previous = line
        if channels == 4:
            rgb = bytearray(width * 3)
            rgb[0::3] = line[0::4]
            rgb[1::3] = line[1::4]
            rgb[2::3] = line[2::4]
            rows.append(rgb)
        else:
            rows.append(line)
    return width, height, rows


def load(path):
    try:
        with open(path, "rb") as file:
            return decode(file.read())
    except OSError as error:
        raise PngError(f"{path}: {error.strerror}")
    except PngError as error:
        raise PngError(f"{path}: {error}")


def measure(first, second, levels):
    """Returns (mean, share percent, max) for two decoded images of equal size."""
    width, height, first_rows = first
    _, _, second_rows = second
    total = 0
    largest = 0
    over = 0
    for a, b in zip(first_rows, second_rows):
        if a == b:
            continue
        differences = [abs(x - y) for x, y in zip(a, b)]
        total += sum(differences)
        largest = max(largest, max(differences))
        for i in range(0, len(differences), 3):
            if max(differences[i : i + 3]) > levels:
                over += 1
    pixels = width * height
    if pixels == 0:
        return 0.0, 0.0, 0
    return total / (pixels * 3), 100.0 * over / pixels, largest


def compare(first, second, max_mean, max_share, levels):
    """Returns (line, exit code)."""
    if first[:2] != second[:2]:
        return f"size mismatch | {first[0]}x{first[1]} vs {second[0]}x{second[1]}", 1
    mean, share, largest = measure(first, second, levels)
    line = f"mean {mean:.4f} | share {share:.4f} % | max {largest} | size {first[0]}x{first[1]}"
    return line, 0 if mean <= max_mean and share <= max_share else 1


def make_png(width, height, filtered_rows, color_type=2):
    """Builds a PNG from rows that already start with their filter-type byte."""
    def chunk(kind, body):
        crc = zlib.crc32(kind + body) & 0xFFFFFFFF
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", crc)

    header = struct.pack(">IIBBBBB", width, height, 8, color_type, 0, 0, 0)
    return (
        SIGNATURE
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(b"".join(filtered_rows)))
        + chunk(b"IEND", b"")
    )


def flat_png(width, height, pixels):
    """A filter-0 RGB PNG whose pixel (x, y) is pixels[y * width + x]."""
    rows = []
    for y in range(height):
        row = bytearray([0])
        for x in range(width):
            row += bytes(pixels[y * width + x])
        rows.append(bytes(row))
    return make_png(width, height, rows)


def self_test():
    import os
    import tempfile

    def identical_images_have_zero_mean_and_exit_zero():
        image = decode(flat_png(2, 2, [(1, 2, 3), (4, 5, 6), (7, 8, 9), (10, 11, 12)]))
        line, code = compare(image, image, DEFAULT_MAX_MEAN, DEFAULT_MAX_SHARE, DEFAULT_LEVELS)
        assert code == 0 and line.startswith("mean 0.0000 "), line
        assert line.endswith("size 2x2"), line

    def each_filter_type_decodes_to_the_same_pixels():
        # Two rows of three RGB pixels, filtered by hand with each type, bpp = 3.
        first = bytes([10, 20, 30, 40, 50, 60, 70, 80, 90])
        second = bytes([15, 25, 35, 100, 10, 200, 71, 82, 93])
        expected = [bytearray(first), bytearray(second)]
        bpp = 3

        def filtered(kind):
            rows = []
            for y, row in enumerate((first, second)):
                up = first if y == 1 else bytes(9)
                out = bytearray([kind])
                for i in range(9):
                    left = row[i - bpp] if i >= bpp else 0
                    above = up[i]
                    corner = up[i - bpp] if i >= bpp else 0
                    predicted = (0, left, above, (left + above) >> 1, paeth(left, above, corner))[kind]
                    out.append((row[i] - predicted) & 255)
                rows.append(bytes(out))
            return rows

        for kind in range(5):
            width, height, rows = decode(make_png(3, 2, filtered(kind)))
            assert (width, height) == (3, 2)
            assert rows == expected, f"filter {kind}: {rows}"
        # RGBA drops alpha.
        rgba = make_png(1, 1, [bytes([0, 9, 8, 7, 255])], color_type=6)
        assert decode(rgba)[2] == [bytearray([9, 8, 7])]

    def a_difference_at_the_level_threshold_is_not_counted():
        first = decode(flat_png(2, 1, [(10, 10, 10), (0, 0, 0)]))
        second = decode(flat_png(2, 1, [(12, 10, 10), (0, 0, 0)]))
        mean, share, largest = measure(first, second, 2)
        assert share == 0.0 and largest == 2, (share, largest)
        assert abs(mean - 2 / 6) < 1e-9, mean

    def a_difference_above_the_level_threshold_is_counted_in_share():
        first = decode(flat_png(2, 1, [(10, 10, 10), (0, 0, 0)]))
        second = decode(flat_png(2, 1, [(13, 10, 10), (0, 0, 0)]))
        _, share, largest = measure(first, second, 2)
        assert share == 50.0 and largest == 3, (share, largest)

    def different_sizes_exit_one():
        small = decode(flat_png(1, 1, [(0, 0, 0)]))
        wide = decode(flat_png(2, 1, [(0, 0, 0), (0, 0, 0)]))
        assert compare(small, wide, 1e9, 1e9, 2)[1] == 1

    def exceeding_either_limit_exits_one():
        first = decode(flat_png(2, 1, [(0, 0, 0), (0, 0, 0)]))
        second = decode(flat_png(2, 1, [(100, 0, 0), (0, 0, 0)]))
        # mean = 100 / 6, share = 50 %.
        assert compare(first, second, 20, 60, 2)[1] == 0
        assert compare(first, second, 10, 60, 2)[1] == 1
        assert compare(first, second, 20, 40, 2)[1] == 1

    def undecodable_files_are_reported():
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "bad.png")
            with open(path, "wb") as file:
                file.write(b"not a png")
            try:
                load(path)
            except PngError:
                return
            raise AssertionError("expected PngError")

    cases = [
        identical_images_have_zero_mean_and_exit_zero,
        each_filter_type_decodes_to_the_same_pixels,
        a_difference_at_the_level_threshold_is_not_counted,
        a_difference_above_the_level_threshold_is_counted_in_share,
        different_sizes_exit_one,
        exceeding_either_limit_exits_one,
        undecodable_files_are_reported,
    ]
    for case in cases:
        case()
        print(f"ok {case.__name__}")
    return 0


def main():
    parser = argparse.ArgumentParser(description="Compare two PNG renders.")
    parser.add_argument("first", nargs="?")
    parser.add_argument("second", nargs="?")
    parser.add_argument("--max-mean", type=float, default=DEFAULT_MAX_MEAN)
    parser.add_argument("--max-share", type=float, default=DEFAULT_MAX_SHARE,
                        help="limit on the percent of pixels over --levels")
    parser.add_argument("--levels", type=int, default=DEFAULT_LEVELS)
    parser.add_argument("--report", help="also write the output line to this file")
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if arguments.self_test:
        return self_test()
    if not arguments.first or not arguments.second:
        parser.error("two PNG files are required")
    try:
        first = load(arguments.first)
        second = load(arguments.second)
    except PngError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    line, code = compare(first, second, arguments.max_mean, arguments.max_share, arguments.levels)
    print(line)
    if arguments.report:
        with open(arguments.report, "w") as file:
            file.write(line + "\n")
    return code


if __name__ == "__main__":
    sys.exit(main())
