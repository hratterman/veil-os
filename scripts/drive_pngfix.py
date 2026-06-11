#!/usr/bin/env python3
"""Regression for the large-PNG OOM crash + downscale-on-decode.

Two oversized PNGs are staged on the disk. AAA2048.PNG (2048x2048) sorts first,
so the viewer opens straight onto it. Before the fix that allocated tens of MiB
on the 16 MiB kernel heap, OOM-panicked, and called semihosting::exit — QEMU
vanished, "OS gone".

After the fix png::decode() streams the scanlines (never holding the full
image) and downscales the output to fit the heap, so the 2048x2048 image
actually RENDERS (smaller). A genuinely over-cap image (ZZHUGE.PNG, 3000x2000)
is declined with a friendly on-screen message. Throughout, the OS stays alive
and every QMP command keeps working.
"""
import sys

from guilib import Driver, check, finish

VIEWER_BTN = (496, 768 - 20)
WIN_X, WIN_Y, CW, CH = 220, 80, 560, 460
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22


def palette(img):
    """Distinct colors sampled across the viewer content box."""
    seen = set()
    for y in range(CONTENT_Y, CONTENT_Y + CH, 6):
        for x in range(CONTENT_X, CONTENT_X + CW, 6):
            seen.add(img.at(x, y))
    return seen


# Decoding a 2048x2048 image streams ~12 MB through the inflate closure in the
# debug build under TCG, which takes a few seconds — keep timeouts generous.
DECODE_T = 25


def arrow(d, key, mark, needle, label):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": key}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": key}}}])
    check(label, d.wait_serial(needle, DECODE_T, mark))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("VIEWER_OK on serial", "VIEWER_OK" in d.serial())

    # Launch the viewer — it opens on AAA2048.PNG (2048x2048), the exact case
    # that used to take the OS down.
    mark = len(d.serial())
    d.click(*VIEWER_BTN)
    check("viewer launched", d.wait_serial("WM: launch 'viewer'", 5, mark))

    # The big image must DECODE (downscaled), not crash and not be refused.
    check("2048x2048 decoded by downscaling (not crashed/refused)",
          d.wait_serial("PNG: downscaled 2048x2048", DECODE_T, mark))
    check("viewer reports the downscale",
          d.wait_serial("VIEWER: showing AAA2048.PNG 2048x2048 (downscaled", DECODE_T, mark))
    check("PNG_CRASH_FIXED emitted", d.wait_serial("PNG_CRASH_FIXED", DECODE_T, mark))

    # It actually rendered: a gradient fills the window with many colors.
    d.move(1000, 700)
    img = d.dump("pngfix_2048")
    check("big image rendered (gradient on screen)", len(palette(img)) >= 20,
          f"{len(palette(img))} distinct colors")

    # A normal image still decodes+renders right after — OS fully alive.
    mark = len(d.serial())
    arrow(d, "right", mark, "VIEWER: showing CHECK.PNG", "right arrow -> CHECK.PNG decodes")
    d.move(1000, 700)
    img2 = d.dump("pngfix_normal")
    check("normal image renders after the big one", len(palette(img2)) >= 2,
          f"{len(palette(img2))} distinct colors")

    # Wrap left from CHECK.PNG... back to the big one, exercising decode again.
    mark = len(d.serial())
    arrow(d, "left", mark, "PNG: downscaled 2048x2048",
          "left arrow -> back to big image, downscales again")

    # Now reach the over-cap image (sorts last): from AAA2048 (idx 0), Left
    # wraps to ZZHUGE.PNG. It must be declined gracefully with the real dims.
    mark = len(d.serial())
    arrow(d, "left", mark, "VIEWER: cannot decode ZZHUGE.PNG (3000x2000",
          "over-cap image declined gracefully with dims")
    d.move(1000, 700)
    d.dump("pngfix_toolarge")  # the on-screen "too large" message

    check("kernel never panicked", "KERNEL PANIC" not in d.serial())
    d.quit()
    finish()


if __name__ == "__main__":
    main()
