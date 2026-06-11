#!/usr/bin/env python3
"""M35: the image viewer displays a JPEG (decoded by the kernel's own decoder).
Open the viewer, step to a .JPG, confirm the window shows real photo content
(many distinct colours), and screenshot it."""
import sys

from guilib import Driver, check, finish

VIEWER_BTN = (496, 768 - 20)
WIN_X, WIN_Y, CW, CH = 220, 80, 560, 460
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22


def palette(img):
    seen = set()
    for y in range(CONTENT_Y, CONTENT_Y + CH, 6):
        for x in range(CONTENT_X, CONTENT_X + CW, 6):
            seen.add(img.at(x, y))
    return seen


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("VIEWER_OK on serial", "VIEWER_OK" in d.serial())
    mark = len(d.serial())
    d.click(*VIEWER_BTN)
    check("viewer launched", d.wait_serial("WM: launch 'viewer'", 5, mark))

    # Step right until the viewer reports showing a .JPG file.
    got_jpg = False
    for _ in range(12):
        if "VIEWER: showing" in d.serial() and ".JPG" in d.serial().split("VIEWER: showing")[-1][:40]:
            got_jpg = True
            break
        mk = len(d.serial())
        d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "right"}}}])
        d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "right"}}}])
        d.wait_serial("VIEWER: showing", 5, mk)
    check("viewer landed on a .JPG", got_jpg or ".JPG" in d.serial())

    d.move(1000, 700)
    img = d.dump("m35_jpeg")
    check("JPEG photo rendered (many colours)", len(palette(img)) >= 30,
          f"{len(palette(img))} distinct colours")
    d.quit()
    finish()


main()
