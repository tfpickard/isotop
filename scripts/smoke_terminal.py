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

# Views reached through the `v` picker after the fixed steps. Each later view adds its name here;
# names this platform does not list in `--list-views` are skipped.
PICKER_VISITS = [
    "reef",
    "ptolemy",
    "tesseract",
    "necropolis",
    "cortex",
    "horizon",
    "murmuration",
    "arbor",
    "anthill",
    "plumbing",
    # Last, so that the live run's final `q` is pressed on the board and plays its farewell.
    "ouija",
]

# A view's own keys, sent right after the picker shows it, each with what the terminal must show
# afterwards before the next keys go out.
VIEW_KEYS = {
    "tesseract": [(b".;", [b"plane YW"]), (b"b\x1b", [b"/ TESSERACT /"])],
}


# Characters that need Shift on a US keyboard, with the key that carries them.
SHIFTED_KEYS = {"+": "=", "*": "8", "?": "/"}

# What Shift+Space is: a CSI u sequence under the Kitty keyboard protocol, otherwise the `*`
# fallback, because a legacy terminal sends Space for it.
SHIFT_SPACE = b"\x1b[32;2u"
# Reporting every key also reports the modifier on its own first: the press of Left Shift.
SHIFT_PRESS = b"\x1b[57441;2u"
CALLOUT_PID = re.compile(rb"\| pid (\d+)")
KEYBOARD_PUSH = b"\x1b[>13u"
KEYBOARD_POP = b"\x1b[<1u"
LEAVE_ALTERNATE_SCREEN = b"\x1b[?1049l"


def kitty_encoded(keys):
    """What a terminal with the keyboard flags isotop pushes (disambiguate, alternate keys and all
    keys as escape codes) sends for the legacy bytes `keys`: every key as `CSI code u`, with the
    shifted character after a colon when Shift makes one."""
    out = b""
    index = 0
    while index < len(keys):
        for raw in (SHIFT_PRESS, SHIFT_SPACE):
            if keys[index:].startswith(raw):
                out += raw
                index += len(raw)
                break
        else:
            raw = None
        if raw is not None:
            continue
        if keys[index : index + 3] == b"\x1b[Z":
            out += b"\x1b[9;2u"
            index += 3
            continue
        code = keys[index]
        index += 1
        character = chr(code)
        if code == 0x1B:
            out += b"\x1b[27u"
        elif character in "\r\t ":
            out += b"\x1b[%du" % code
        elif character.isupper():
            out += b"\x1b[%d:%d;2u" % (ord(character.lower()), code)
        elif character in SHIFTED_KEYS:
            out += b"\x1b[%d:%d;2u" % (ord(SHIFTED_KEYS[character]), code)
        else:
            out += b"\x1b[%du" % code
    return out


class NewCallout:
    """Satisfied once the tour callout names a process other than the one shown at the step."""

    def __init__(self, before):
        self.before = before

    def __call__(self, received):
        return any(int(pid) != self.before for pid in CALLOUT_PID.findall(received))


class MarkScanner:
    """Lists the keyboard push and pop and the alternate screen exit in the order the terminal
    output carried them, whichever reads they were split across."""

    MARKS = (KEYBOARD_PUSH, KEYBOARD_POP, LEAVE_ALTERNATE_SCREEN)

    def __init__(self):
        self.marks = []
        self.carry = b""

    def feed(self, data):
        window = self.carry + data
        found = []
        for mark in self.MARKS:
            start = window.find(mark)
            while start >= 0:
                # A mark that ends inside the carried bytes was counted when they were new.
                if start + len(mark) > len(self.carry):
                    found.append((start, mark))
                start = window.find(mark, start + 1)
        self.marks.extend(mark for _, mark in sorted(found))
        self.carry = window[-(max(map(len, self.MARKS)) - 1) :]


def listed_views(binary):
    """The view names `binary --list-views` prints, which are the ones this platform supports."""
    result = subprocess.run([binary, "--list-views"], capture_output=True, text=True, check=True)
    return result.stdout.split()


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


def exercise(binary, demo, shared_memory, keyboard):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 120, 960, 576))
    command = [binary, "--width", "320", "--fps", "10", "--sample-ms", "100"]
    if demo:
        command.append("--demo")
    supported = listed_views(binary)
    visits = [name for name in PICKER_VISITS if name in supported]
    # Each key step waits for the header it should produce before the next keys go out, so two
    # steps can never land before a frame is drawn between them. `since_sent` holds what the
    # terminal received after the latest step, and a step is done once it holds every needle.
    # Shift+Tab from city wraps to the last view; the views registered after coop come first,
    # then coop, and Tab goes forward through them again to wrap to city.
    after_coop = supported[supported.index("coop") + 1 :]

    def header(name):
        return f"/ {name.upper()} /".encode()

    steps = [(b"\x1b[Z", [header(name)]) for name in reversed(after_coop)]
    steps += [(b"\x1b[Z", [b"/ COOP /"])]
    steps += [(b"\t", [header(name)]) for name in after_coop]
    steps += [
        (b"\t", [b"/ CITY /"]),
        (b"\t", [b"/ ORBIT /"]),
        # Space visits the next process and the tour goes on; with the tour off the visit holds
        # still and the strip says LIVE. Shift+Space (or `*`) jumps to another process, never the
        # one shown, and P pauses.
        (b"g", [b"/ TOUR"]),
        (b" ", [NewCallout, b"/ TOUR"]),
        # Under the keyboard protocol Shift is reported on its own just before Shift+Space, and
        # that press must not end the tour: the strip still says TOUR after the jump.
        (SHIFT_PRESS + SHIFT_SPACE if keyboard else b"*", [NewCallout, b"/ TOUR"]),
        (b"g", [b"/ LIVE"]),
        (b" ", [NewCallout, b"/ LIVE"]),
        (SHIFT_SPACE if keyboard else b"*", [NewCallout, b"/ LIVE"]),
        (b" ", [NewCallout, b"/ LIVE"]),
        (b"P", [b"/ PAUSED"]),
        (b"P", [b"/ LIVE"]),
        (b"/worker\r+efP\x1b", [b"/ PAUSED"]),
        (b"7Pg", [b"/ STRATA /", b"/ TOUR"]),
    ]
    for name in visits:
        steps.append((b"v" + name.encode() + b"\r", [f"/ {name.upper()} /".encode()]))
        steps += VIEW_KEYS.get(name, [])
    view_keys = sum(len(VIEW_KEYS.get(name, [])) for name in visits)
    required_frames = 16 + 2 * len(visits) + view_keys
    time_limit = 30 + 5 * len(visits) + 2 * len(after_coop) + 2 * view_keys
    child = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave)
    buffer = b""
    payload = b""
    frame_count = 0
    queries = 0
    decoded = []
    shared_names = set()
    headers = set()
    step = 0
    awaiting = []
    since_sent = b""
    last_pid = None
    scanner = MarkScanner()
    picker_headers = [f"/ {name.upper()} /".encode() for name in visits]
    started = time.monotonic()
    try:
        while time.monotonic() - started < time_limit:
            if select.select([master], [], [], 0.1)[0]:
                try:
                    data = os.read(master, 262144)
                except OSError as error:
                    if error.errno == errno.EIO:
                        break
                    raise
                buffer += data
                scanner.feed(data)
                since_sent = (since_sent + data)[-(1 << 20) :]
                for header in [b"/ CITY /", b"/ ORBIT /", b"/ STRATA /", b"/ COOP /", b"/ PAUSED", b"/ TOUR"] + picker_headers:
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
                    # Answers come in query order: the keyboard flags, then the attributes.
                    reply = b"\x1b[?0u" if keyboard and b"\x1b[?u" in data else b""
                    os.write(master, reply + b"\x1b[?62;22c")
            if awaiting and all(
                needle(since_sent) if callable(needle) else needle in since_sent
                for needle in awaiting
            ):
                awaiting = []
            if not awaiting and frame_count >= 1 and step < len(steps):
                keys, awaiting = steps[step]
                step += 1
                pids = CALLOUT_PID.findall(since_sent)
                last_pid = int(pids[-1]) if pids else last_pid
                awaiting = [NewCallout(last_pid) if needle is NewCallout else needle for needle in awaiting]
                since_sent = b""
                os.write(master, kitty_encoded(keys) if keyboard else keys)
            if not awaiting and step == len(steps) and frame_count >= required_frames:
                # Ctrl-C quits where the keyboard protocol sends it as `CSI 99;5u`.
                os.write(master, b"\x1b[99;5u" if keyboard else b"q")
                break
        if child.poll() is None and (awaiting or step < len(steps)):
            child.kill()
            raise AssertionError(f"step {step} of {len(steps)} ({steps[step - 1][0]!r}) never saw {awaiting}")
        # Keep reading until the child exits: a pseudo-terminal with a small buffer (macOS) blocks
        # the child's writes once it fills, and then it never reaches its exit path.
        deadline = time.monotonic() + 5
        while child.poll() is None and time.monotonic() < deadline:
            if select.select([master], [], [], 0.05)[0]:
                try:
                    data = os.read(master, 262144)
                    scanner.feed(data)
                    buffer += data
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
            scanner.feed(data)
            buffer += data
        for match in re.finditer(rb"\x1b_G([^;\x1b]*);([A-Za-z0-9+/=]*)\x1b\\", buffer):
            if b"t=s" in match.group(1).split(b","):
                try:
                    shared_names.add(base64.b64decode(match.group(2)).decode())
                except (ValueError, UnicodeDecodeError):
                    pass
        assert child.returncode == 0, f"exit status {child.returncode}: {buffer!r}"
        assert queries == 1, "capability detection was not exercised"
        assert step == len(steps) and not awaiting, f"step {step} of {len(steps)} never saw {awaiting}"
        assert frame_count >= required_frames, f"only {frame_count} frames"
        assert len(set(decoded)) >= 2, "frames never changed"
        assert b"/ CITY /" in headers and b"/ ORBIT /" in headers, headers
        assert b"/ COOP /" in headers, "Shift+Tab did not switch to the coop view"
        assert b"/ PAUSED" in headers, "pause did not take effect"
        assert b"/ TOUR" in headers, "g did not start the tour"
        assert b"/ STRATA /" in headers, "7 did not switch to the strata view"
        for name, header in zip(visits, picker_headers):
            assert header in headers, f"the picker did not switch to the {name} view"
        assert not (termios.tcgetattr(slave)[3] & termios.ICANON) == 0, "raw mode was not restored"
        if keyboard:
            # The flags are pushed once and popped before the alternate screen is left, since each
            # screen keeps its own stack.
            assert scanner.marks == [KEYBOARD_PUSH, KEYBOARD_POP, LEAVE_ALTERNATE_SCREEN], scanner.marks
        else:
            assert scanner.marks == [LEAVE_ALTERNATE_SCREEN], scanner.marks
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
        print(f"{'demo' if demo else 'live'}: {frame_count} valid RGB frames via {transport}; query, view switch, back-tab to coop, search, focus, pause, number keys, tour, Space and {'Shift+Space' if keyboard else '*'} visits, P, picker visits ({', '.join(visits) or 'none'}), view keys ({view_keys} steps), quit, terminal restoration passed")
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
    # The demo run answers the Kitty keyboard query and speaks the protocol; the live run does not.
    exercise(os.path.abspath(options.binary), True, True, True)
    exercise(os.path.abspath(options.binary), False, False, False)
