#!/usr/bin/env python3
"""The user's exact scenario: a real-world 1920x1080 PNG (staged the way a
session-manager upload stages it, via mkdisk --extra-dir) opened in the viewer
must NOT take the OS down. Confirms PNG_CRASH_FIXED + a real render + no panic."""
import re
import sys
from guilib import Driver, check, finish


def viewer_btn(d):
    hits = re.findall(r"TASKBAR_PILL: viewer (\d+) (\d+)", d.serial())
    if not hits:
        return (496, 768 - 20)
    x, w = int(hits[-1][0]), int(hits[-1][1])
    return (x + w // 2, 768 - 20)


WIN_X, WIN_Y, CW, CH = 220, 80, 560, 460
CX, CY = WIN_X + 2, WIN_Y + 2 + 22


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("VIEWER_OK on serial", "VIEWER_OK" in d.serial())
    mark = len(d.serial())
    d.click(*viewer_btn(d))
    check("viewer launched", d.wait_serial("WM: launch 'viewer'", 5, mark))
    # AAA1920.PNG sorts first, so the viewer opens straight onto it.
    check("1920x1080 decoded without crash", d.wait_serial("VIEWER: showing AAA1920.PNG", 25, mark))
    check("PNG_CRASH_FIXED emitted", d.wait_serial("PNG_CRASH_FIXED", 25, mark))
    d.move(1000, 700)
    img = d.dump("png1920")
    seen = set()
    for y in range(CY, CY + CH, 6):
        for x in range(CX, CX + CW, 6):
            seen.add(img.at(x, y))
    check("image actually rendered (many colors)", len(seen) >= 20, f"{len(seen)} colors")
    check("kernel never panicked", "KERNEL PANIC" not in d.serial() and "FATAL" not in d.serial())
    d.quit()
    finish()


if __name__ == "__main__":
    main()
