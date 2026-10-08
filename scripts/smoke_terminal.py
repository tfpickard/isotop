#!/usr/bin/env python3
"""Exercise terminal output and interaction through a pseudo-terminal."""

import argparse
import base64
import errno
import fcntl
import os
import pty
import select
import struct
import subprocess
import termios
import time
import zlib


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
    headers = set()
    commands_sent = False
    tour_sent = False
    started = time.monotonic()
    try:
        while time.monotonic() - started < 8:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    data = os.read(master, 262144)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                buffer += data
                for header in [b"/ CITY /", b"/ ORBIT /", b"/ PAUSED", b"/ TOUR"]:
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
                        elif medium == b"s" and shared_memory:
                            os.write(master, b"\x1b_Gi=32;OK\x1b\\")
                    elif medium == b"s":
                        # Like Kitty and Ghostty: read the object named in the payload, then unlink it.
                        path = "/dev/shm/" + base64.b64decode(body).decode().lstrip("/")
                        with open(path, "rb") as shared:
                            pixels = shared.read()
                        os.unlink(path)
                        assert len(pixels) == int(fields[b"s"]) * int(fields[b"v"]) * 3, "incorrect shared-memory size"
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
            if frame_count >= 2 and not commands_sent:
                os.write(master, b"\t/worker\r+ef \x1b")
                commands_sent = True
            if frame_count >= 5 and commands_sent and not tour_sent:
                os.write(master, b" g")
                tour_sent = True
            if frame_count >= 8:
                os.write(master, b"q")
                break
        child.wait(timeout=3)
        assert child.returncode == 0, f"exit status {child.returncode}: {buffer!r}"
        assert queries == 1, "capability detection was not exercised"
        assert frame_count >= 8, f"only {frame_count} frames"
        assert len(set(decoded)) >= 2, "frames never changed"
        assert b"/ CITY /" in headers and b"/ ORBIT /" in headers, headers
        assert b"/ PAUSED" in headers, "pause did not take effect"
        assert b"/ TOUR" in headers, "g did not start the tour"
        assert not (termios.tcgetattr(slave)[3] & termios.ICANON) == 0, "raw mode was not restored"
        leftovers = [name for name in os.listdir("/dev/shm") if name.startswith(f"isotop-{child.pid}-")]
        assert not leftovers, f"shared memory left behind: {leftovers}"
        transport = "shared memory" if shared_memory else "inline zlib"
        print(f"{'demo' if demo else 'live'}: {frame_count} valid RGB frames via {transport}; query, view switch, search, focus, pause, tour, quit, terminal restoration passed")
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
