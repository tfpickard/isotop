#!/usr/bin/env python3
"""Exercise terminal output and interaction through a pseudo-terminal."""

import argparse
import base64
import ctypes
import ctypes.util
import errno
import fcntl
import mmap
import os
import pty
import re
import select
import struct
import subprocess
import sys
import termios
import time
import zlib

ON_LINUX = sys.platform.startswith("linux")


def load_shared_memory_functions():
    """Binds shm_open and shm_unlink, which Linux and macOS both provide through libc.

    glibc before 2.34 keeps them in librt, so that library is the fallback.
    """
    for library_name in (None, ctypes.util.find_library("c"), ctypes.util.find_library("rt")):
        try:
            library = ctypes.CDLL(library_name, use_errno=True)
            opener, remover = library.shm_open, library.shm_unlink
        except (OSError, AttributeError):
            continue
        # shm_open is variadic in C (the optional third argument is the creation mode). Without
        # O_CREAT the mode is never read, so declaring only two arguments is safe even though
        # the arm64 variadic convention passes extra arguments on the stack.
        opener.argtypes = [ctypes.c_char_p, ctypes.c_int]
        opener.restype = ctypes.c_int
        remover.argtypes = [ctypes.c_char_p]
        remover.restype = ctypes.c_int
        return opener, remover
    raise RuntimeError("shm_open is not available in libc")


shm_open, shm_unlink = load_shared_memory_functions()


def shared_memory_exists(name):
    """Whether the POSIX shared-memory object `name` (with its leading slash) can be opened."""
    descriptor = shm_open(name.encode(), os.O_RDONLY)
    if descriptor < 0:
        error = ctypes.get_errno()
        assert error == errno.ENOENT, f"shm_open({name}) failed with {os.strerror(error)}"
        return False
    os.close(descriptor)
    return True


def read_shared_frame(name, length):
    """Reads `length` bytes from a shared-memory object the way a terminal does, then unlinks it.

    The object may be larger than the frame (some systems round the size up to a page).
    """
    descriptor = shm_open(name.encode(), os.O_RDONLY)
    if descriptor < 0:
        error = ctypes.get_errno()
        raise OSError(error, f"shm_open({name}): {os.strerror(error)}")
    try:
        size = os.fstat(descriptor).st_size
        assert size >= length, f"shared memory holds {size} bytes, expected {length}"
        if ON_LINUX:
            assert size == length, f"shared memory holds {size} bytes, expected {length}"
        with mmap.mmap(descriptor, length, flags=mmap.MAP_SHARED, prot=mmap.PROT_READ) as mapping:
            pixels = mapping[:length]
    finally:
        os.close(descriptor)
    shm_unlink(name.encode())
    return pixels


def exercise(binary, demo, shared_memory):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 120, 960, 576))
    command = [binary, "--width", "320", "--fps", "10", "--sample-ms", "100"]
    if demo:
        command.append("--demo")
    child = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave)
    buffer = b""
    payload = b""
    frame_count = 0
    queries = 0
    decoded = []
    shared_names = set()
    headers = set()
    back_tab_sent = False
    coop_left = False
    commands_sent = False
    tour_sent = False
    started = time.monotonic()
    try:
        while time.monotonic() - started < 20:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    data = os.read(master, 262144)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                buffer += data
                for header in [b"/ CITY /", b"/ ORBIT /", b"/ STRATA /", b"/ COOP /", b"/ PAUSED", b"/ TOUR"]:
                    if header in data:
                        headers.add(header)
                while b"\x1b_G" in buffer:
                    start = buffer.index(b"\x1b_G")
                    end = buffer.find(b"\x1b\\", start)
                    if end < 0:
                        break
                    packet = buffer[start + 3 : end]
                    buffer = buffer[end + 2 :]
                    control, _, body = packet.partition(b";")
                    fields = dict(field.split(b"=", 1) for field in control.split(b",") if b"=" in field)
                    medium = fields.get(b"t", b"d")
                    if fields.get(b"a") == b"q":
                        if medium == b"d":
                            os.write(master, b"\x1b_Gi=31;OK\x1b\\")
                            queries += 1
                        elif medium == b"s":
                            shared_names.add(base64.b64decode(body).decode())
                            if shared_memory:
                                os.write(master, b"\x1b_Gi=32;OK\x1b\\")
                    elif medium == b"s":
                        # Like Kitty and Ghostty: read the object named in the payload, then unlink it.
                        name = base64.b64decode(body).decode()
                        shared_names.add(name)
                        pixels = read_shared_frame(name, int(fields[b"s"]) * int(fields[b"v"]) * 3)
                        decoded.append(pixels)
                        frame_count += 1
                    elif body:
                        if fields.get(b"a") == b"T":
                            size = int(fields[b"s"]) * int(fields[b"v"]) * 3
                            payload = b""
                        payload += body
                        if fields.get(b"m") == b"0":
                            pixels = zlib.decompress(base64.b64decode(payload))
                            assert len(pixels) == size, "incorrect RGB payload size"
                            decoded.append(pixels)
                            frame_count += 1
                if len(buffer) > 4096 and b"\x1b_G" not in buffer:
                    buffer = buffer[-4096:]
                if b"\x1b[c" in data:
                    os.write(master, b"\x1b[?62;22c")
            if frame_count >= 1 and not back_tab_sent:
                os.write(master, b"\x1b[Z")
                back_tab_sent = True
            if b"/ COOP /" in headers and not coop_left:
                os.write(master, b"\t")
                coop_left = True
            if frame_count >= 2 and coop_left and not commands_sent:
                os.write(master, b"\t/worker\r+ef \x1b")
                commands_sent = True
            if frame_count >= 5 and commands_sent and not tour_sent:
                os.write(master, b"7 g")
                tour_sent = True
            if frame_count >= 8:
                os.write(master, b"q")
                break
        # Keep reading until the child exits: a pseudo-terminal with a small buffer (macOS) blocks
        # the child's writes once it fills, and then it never reaches its exit path.
        deadline = time.monotonic() + 5
        while child.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.05)[0]:
                try:
                    buffer += os.read(master, 262144)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    break
        child.wait(timeout=1)
        # Output the child wrote but this loop never parsed may still name frame objects.
        while select.select([master], [], [], 0)[0]:
            try:
                data = os.read(master, 262144)
            except OSError as error:
                if error.errno != errno.EIO:
                    raise
                break
            if not data:
                break
            buffer += data
        for match in re.finditer(rb"\x1b_G([^;\x1b]*);([A-Za-z0-9+/=]*)\x1b\\", buffer):
            if b"t=s" in match.group(1).split(b","):
                try:
                    shared_names.add(base64.b64decode(match.group(2)).decode())
                except (ValueError, UnicodeDecodeError):
                    pass
        assert child.returncode == 0, f"exit status {child.returncode}: {buffer!r}"
        assert queries == 1, "capability detection was not exercised"
        assert frame_count >= 8, f"only {frame_count} frames"
        assert len(set(decoded)) >= 2, "frames never changed"
        assert b"/ CITY /" in headers and b"/ ORBIT /" in headers, headers
        assert b"/ COOP /" in headers, "Shift+Tab did not switch to the coop view"
        assert b"/ PAUSED" in headers, "pause did not take effect"
        assert b"/ TOUR" in headers, "g did not start the tour"
        assert b"/ STRATA /" in headers, "7 did not switch to the strata view"
        assert not (termios.tcgetattr(slave)[3] & termios.ICANON) == 0, "raw mode was not restored"
        if ON_LINUX:
            leftovers = [name for name in os.listdir("/dev/shm") if name.startswith(f"isotop-{child.pid}-")]
        else:
            # macOS has no directory to list. Every name the terminal was told about must be gone,
            # and so must every name the app could have made: serials count up from zero, so probe
            # up to the highest one seen plus a margin, and the capability probe's object.
            prefix = f"/isotop-{child.pid}-"
            serials = [int(name[len(prefix) :]) for name in shared_names if re.fullmatch(re.escape(prefix) + r"\d+", name)]
            candidates = set(shared_names)
            candidates.update(f"{prefix}{serial}" for serial in range(max(serials, default=0) + 17))
            candidates.add(f"{prefix}probe")
            leftovers = [name for name in sorted(candidates) if shared_memory_exists(name)]
            for name in leftovers:
                shm_unlink(name.encode())
        assert not leftovers, f"shared memory left behind: {leftovers}"
        transport = "shared memory" if shared_memory else "inline zlib"
        print(f"{'demo' if demo else 'live'}: {frame_count} valid RGB frames via {transport}; query, view switch, back-tab to coop, search, focus, pause, number keys, tour, quit, terminal restoration passed")
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(master)
        os.close(slave)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", nargs="?", default="target/release/isotop")
    options = parser.parse_args()
    exercise(os.path.abspath(options.binary), True, True)
    exercise(os.path.abspath(options.binary), False, False)
