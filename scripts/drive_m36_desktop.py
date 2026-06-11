#!/usr/bin/env python3
"""M36 desktop features: Print Screen screenshot + toast, right-click desktop
context menu, and Change Wallpaper."""
import sys

from guilib import Driver, check, finish


def rclick(d, x, y):
    d.move(x, y)
    d.send([{"type": "btn", "data": {"down": True, "button": "right"}}])
    d.send([{"type": "btn", "data": {"down": False, "button": "right"}}])


def key(d, qcode):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK", "WM_OK" in d.serial())

    # 1. Print Screen key -> screenshot to disk + toast.
    m = len(d.serial())
    key(d, "print")
    ok = d.wait_serial("SCREENSHOT_OK:", 6, m)
    check("Print Screen saves screenshot", ok)
    check("screenshot toast", d.wait_serial("NOTIFY: Screenshot saved", 3, m))

    # 2. Right-click empty desktop -> context menu, click "Screenshot" item.
    m = len(d.serial())
    rclick(d, 600, 360)
    # menu items: New File / New Folder / Screenshot / Change Wallpaper / Settings
    # anchored at (600,360); item i at y = 360+4+i*26. Screenshot = i=2 -> y=416.
    d.click(640, 416)
    check("desktop menu Screenshot", d.wait_serial("SCREENSHOT_OK:", 6, m))

    # 3. Right-click -> Change Wallpaper (i=3 -> y=442).
    m = len(d.serial())
    rclick(d, 600, 360)
    d.click(640, 442)
    check("change wallpaper", d.wait_serial("Wallpaper changed", 4, m) or d.wait_serial("WALLPAPER:", 4, m))

    d.move(500, 300)
    d.dump("m36_desktop")
    d.quit()
    finish()


main()
