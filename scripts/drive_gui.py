#!/usr/bin/env python3
"""M6/M7/M8 behavioral proof: drive the OS through QEMU's QMP input
injection and verify every pass criterion from screendumps + serial logs.

Geometry/color constants mirror src/wm.rs and src/desktop.rs.

Usage: drive_gui.py <qmp-socket> <serial-log> <shots-dir>
"""
import json
import socket
import sys
import time

W, H = 1024, 768
ABS_MAX = 32767

# Colors (RGB) — must match wm.rs.
PURPLE = (144, 80, 192)       # beta static canvas
T_FOCUS = (48, 96, 192)       # focused title bar
T_UNFOCUS = (112, 120, 128)   # unfocused title bar
DESKTOP = (40, 72, 88)
RED = (224, 48, 48)           # palette[1]
BLUE = (48, 96, 224)          # palette[3]
WHITE = (255, 255, 255)
BLACK = (0, 0, 0)
ECHO_TEXT = (32, 40, 64)

failures = []


class Qmp:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(path)
        self.f = self.sock.makefile("rw")
        self._recv()  # greeting
        self.cmd("qmp_capabilities")

    def _recv(self):
        while True:
            msg = json.loads(self.f.readline())
            if "event" not in msg:
                return msg

    def cmd(self, name, **args):
        self.f.write(json.dumps({"execute": name, "arguments": args}) + "\n")
        self.f.flush()
        resp = self._recv()
        if "error" in resp:
            raise RuntimeError(f"{name}: {resp['error']}")
        return resp


def to_abs(px, range_px):
    return round(px * ABS_MAX / (range_px - 1))


class Driver:
    def __init__(self, qmp, serial_path, shots):
        self.q = qmp
        self.serial_path = serial_path
        self.shots = shots

    def send(self, events):
        self.q.cmd("input-send-event", events=events)
        time.sleep(0.06)

    def move(self, x, y):
        self.send([
            {"type": "abs", "data": {"axis": "x", "value": to_abs(x, W)}},
            {"type": "abs", "data": {"axis": "y", "value": to_abs(y, H)}},
        ])

    def button(self, down):
        self.send([{"type": "btn", "data": {"down": down, "button": "left"}}])

    def click(self, x, y):
        self.move(x, y)
        self.button(True)
        self.button(False)

    def drag(self, x0, y0, x1, y1, steps=8):
        self.move(x0, y0)
        self.button(True)
        for i in range(1, steps + 1):
            self.move(x0 + (x1 - x0) * i // steps, y0 + (y1 - y0) * i // steps)
        self.button(False)

    def type_text(self, text):
        for ch in text:
            qcode = {" ": "spc", "\n": "ret"}.get(ch, ch)
            for down in (True, False):
                self.send([{"type": "key", "data": {
                    "down": down, "key": {"type": "qcode", "data": qcode}}}])

    def dump(self, name):
        time.sleep(0.4)
        ppm = f"{self.shots}/{name}.ppm"
        self.q.cmd("screendump", filename=ppm, format="ppm")
        self.q.cmd("screendump", filename=f"{self.shots}/{name}.png", format="png")
        time.sleep(0.1)
        return Image(ppm)

    def serial(self):
        return open(self.serial_path, errors="replace").read()


class Image:
    def __init__(self, path):
        data = open(path, "rb").read()
        parts = data.split(b"\n", 3)
        assert parts[0] == b"P6" and parts[2] == b"255"
        self.w, self.h = map(int, parts[1].split())
        self.px = parts[3]

    def at(self, x, y):
        i = (y * self.w + x) * 3
        return (self.px[i], self.px[i + 1], self.px[i + 2])


def check(label, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'} {label}{': ' + detail if detail else ''}")
    if not ok:
        failures.append(label)


def check_px(img, label, x, y, want, invert=False):
    got = img.at(x, y)
    ok = (got != want) if invert else (got == want)
    rel = "!=" if invert else "=="
    check(label, ok, f"@({x},{y}) got rgb{got}, want {rel} rgb{want}")


# Taskbar launcher buttons (UX overhaul): x = 70 + idx*78 + 36, bottom strip.
def taskbar(idx):
    return (70 + idx * 78 + 36, H - 20)

EDITOR_BTN, CLOCK_BTN, PAINT_BTN = taskbar(0), taskbar(1), taskbar(3)
PARK = (990, 620)  # empty desktop corner: park the cursor clear of snapshots


def launch(d, app, btn):
    mark = len(d.serial())
    d.click(*btn)
    check(f"{app} launched", f"WM: launch '{app}'" in d.serial()[mark:]
          or wait_line(d, f"WM: launch '{app}'", mark))


def wait_line(d, needle, after, timeout=5.0):
    end = time.time() + timeout
    while time.time() < end:
        if needle in d.serial()[after:]:
            return True
        time.sleep(0.1)
    return False


def main():
    qmp_path, serial_path, shots = sys.argv[1], sys.argv[2], sys.argv[3]
    d = Driver(Qmp(qmp_path), serial_path, shots)

    # Capability sentinels still print at boot even though no window opens.
    for sentinel in ["INPUT_OK", "M6_OK", "WM_OK", "M7_OK", "PAINT_OK", "M8_OK"]:
        check(f"serial sentinel {sentinel}", sentinel in d.serial())

    print("--- M6: launch editor, keyboard echo + cursor + click -------")
    launch(d, "edit", EDITOR_BTN)        # editor at (40, 40, 420, 300)
    d.click(150, 150)                    # focus + a click at exact coords
    d.type_text("veil")
    d.move(700, 500)                     # park cursor over the desktop
    img = d.dump("m6")
    log = d.serial()
    check("click detected at exact coords", "CLICK: left down @ (150, 150)" in log)
    for ch in "veil":
        check(f"key '{ch}' received", f"KEY: '{ch}'" in log)
    # Typed text is dark glyphs on the editor's white text area (canvas
    # origin screen (48, 98)); count non-white pixels where "veil" lands.
    lit = sum(1 for y in range(96, 116) for x in range(46, 130)
              if img.at(x, y) != WHITE)
    check("typed text echoed to framebuffer", lit >= 20, f"{lit} glyph pixels")
    check_px(img, "cursor tip", 700, 500, BLACK)
    check_px(img, "cursor body 1", 701, 502, WHITE)
    check_px(img, "cursor body 2", 703, 505, WHITE)
    check_px(img, "cursor edge", 709, 509, BLACK)
    check_px(img, "editor title focused", 200, 52, T_FOCUS)

    print("--- M7: second window, drag, focus, z-order -----------------")
    launch(d, "clock", CLOCK_BTN)        # clock at (700, 36, 260, 260), now on top
    img = d.dump("m7_two")
    check_px(img, "clock (newly launched) title focused", 760, 48, T_FOCUS)
    check_px(img, "editor title unfocused under clock", 200, 52, T_UNFOCUS)
    # Drag the clock by its title bar so its title sits OVER the editor's
    # text area: grab (760, 48) -> drop (300, 195). Origin moves to
    # (300-60, 195-12) = (240, 183). Park the cursor before snapshotting so
    # the pointer sprite doesn't sit on the pixel under test.
    d.drag(760, 48, 300, 195)
    d.move(*PARK)
    img = d.dump("m7_dragged")
    check("clock drag reported", "WM: 'clock' moved to (240, 183)" in d.serial())
    check_px(img, "clock title now over editor (z-order: clock on top)",
             300, 195, T_FOCUS)
    # Raise the editor by clicking its title bar; it must occlude the clock.
    d.click(200, 52)
    d.move(*PARK)
    img = d.dump("m7_raised")
    check("editor raised, title focused", img.at(200, 52) == T_FOCUS)
    check_px(img, "editor content now occludes clock (z-order swapped)",
             300, 195, WHITE)

    print("--- M8: launch paint, strokes, persistence, clear -----------")
    launch(d, "paint", PAINT_BTN)        # paint clamps to (480, 322), content (482,346)
    d.click(526, 360)                    # palette: red (idx 1)
    d.drag(550, 442, 750, 492, steps=6)
    d.click(786, 360)                    # brush: large (idx 2)
    d.click(582, 360)                    # palette: blue (idx 3)
    d.drag(560, 560, 700, 610, steps=6)
    d.move(*PARK)
    img = d.dump("m8_strokes")
    log = d.serial()
    check("red selected", "PAINT: color set to #e03030" in log)
    check("blue selected", "PAINT: color set to #3060e0" in log)
    check("brush size changed", "PAINT: brush radius 7" in log)
    check("strokes logged", log.count("PAINT: stroke start") >= 2
          and log.count("PAINT: stroke end") >= 2)
    check_px(img, "red stroke on canvas", 650, 467, RED)
    check_px(img, "blue stroke (large brush)", 630, 585, BLUE)

    print("--- M8: strokes persist under window traffic -----------------")
    # Drag the editor (grab its title at (200,52)) so its content covers the
    # strokes: drop at (650,312) -> origin (490,300), content y 324..624
    # covers both strokes without hitting the taskbar clamp (max origin y
    # 402). Then grab the title at its new spot (650,312) and move it back.
    d.drag(200, 52, 650, 312)
    d.move(*PARK)
    img = d.dump("m8_covered")
    check_px(img, "red stroke hidden under editor", 650, 467, RED, invert=True)
    d.drag(650, 312, 200, 52)            # move the editor away again
    d.move(*PARK)
    img = d.dump("m8_persist")
    check_px(img, "red stroke persisted", 650, 467, RED)
    check_px(img, "blue stroke persisted", 630, 585, BLUE)

    print("--- M8: clear -------------------------------------------------")
    d.click(934, 360)                    # CLR button
    img = d.dump("m8_clear")
    check("clear logged", "PAINT: cleared" in d.serial())
    check_px(img, "canvas cleared (red gone)", 650, 467, WHITE)
    check_px(img, "canvas cleared (blue gone)", 630, 585, WHITE)

    d.q.cmd("quit")
    print(f"\n{'FAILED: ' + str(len(failures)) + ' checks' if failures else 'ALL GUI CHECKS PASSED'}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
