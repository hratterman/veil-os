#!/usr/bin/env python3
"""Regression for the large-PNG OOM crash.

A real-world-sized PNG (AAABIG.PNG, 1920x1080) is staged on the disk so it
sorts first and the image viewer opens straight onto it. Before the fix this
allocated tens of MiB on the 16 MiB kernel heap, OOM-panicked, and called
semihosting::exit — QEMU vanished, noVNC dropped, "OS gone".

After the fix png::decode() guards on the heap budget and refuses the image
gracefully (the viewer shows "cannot decode"), emitting PNG_CRASH_FIXED. The
OS stays alive: we then navigate to a normal PNG and confirm it still renders,
and every QMP command (screendump, input) keeps working — proof QEMU never
exited.
"""
import sys

from guilib import Driver, check, finish

VIEWER_BTN = (496, 768 - 20)
WIN_X, WIN_Y, CW, CH = 220, 80, 560, 460
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22


def nonblank(img):
    """Distinct colors sampled across the viewer content box."""
    seen = set()
    for y in range(CONTENT_Y, CONTENT_Y + CH, 8):
        for x in range(CONTENT_X, CONTENT_X + CW, 8):
            seen.add(img.at(x, y))
    return seen


def arrow(d, key, mark, needle, label):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": key}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": key}}}])
    check(label, d.wait_serial(needle, 5, mark))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("VIEWER_OK on serial", "VIEWER_OK" in d.serial())

    # Launch the viewer — it opens on AAABIG.PNG (first alphabetically), the
    # giant image that used to take the OS down.
    mark = len(d.serial())
    d.click(*VIEWER_BTN)
    check("viewer launched", d.wait_serial("WM: launch 'viewer'", 5, mark))

    # The decoder must REFUSE it gracefully, not crash.
    check("large PNG refused gracefully (not crashed)",
          d.wait_serial("VIEWER: cannot decode AAABIG.PNG", 6, mark))
    check("PNG_CRASH_FIXED emitted", d.wait_serial("PNG_CRASH_FIXED", 6, mark))

    # OS is still alive: QMP screendump works and shows the viewer window.
    d.move(1000, 700)
    img = d.dump("pngfix_big")
    check("viewer window still rendering (QEMU alive)", len(nonblank(img)) >= 2,
          f"{len(nonblank(img))} distinct colors")

    # Navigate to a normal image and confirm full decode+render still works.
    mark = len(d.serial())
    arrow(d, "right", mark, "VIEWER: showing CHECK.PNG", "right arrow -> CHECK.PNG decodes")
    d.move(1000, 700)
    img2 = d.dump("pngfix_recover")
    check("normal image renders after the big one", len(nonblank(img2)) >= 2,
          f"{len(nonblank(img2))} distinct colors")

    # And back, exercising the decoder once more — still no crash.
    mark = len(d.serial())
    arrow(d, "left", mark, "VIEWER: cannot decode AAABIG.PNG",
          "left arrow -> back to big image, still graceful")

    check("kernel never panicked", "KERNEL PANIC" not in d.serial())
    d.quit()
    finish()


if __name__ == "__main__":
    main()
