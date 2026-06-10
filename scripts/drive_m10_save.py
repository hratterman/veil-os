#!/usr/bin/env python3
"""M10 boot 1: draw a red stroke in Paint and SAVE it to the filesystem."""
import sys
from guilib import Driver, check, check_px, finish

RED = (224, 48, 48)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("M10_OK at boot", "M10_OK" in d.serial())

    # UX overhaul: nothing opens at boot. Launch Paint from the taskbar
    # (idx 3: x=70+3*78+36=340, bottom 40px strip). The window clamps 8px
    # above the taskbar, so toolbar/canvas y sit 8px up vs the old build.
    mark = len(d.serial())
    d.click(340, 768 - 20)
    check("paint launched", d.wait_serial("WM: launch 'paint'", 5, mark))

    d.click(526, 360)             # palette: red
    d.drag(550, 442, 750, 492, steps=6)
    img = d.dump("m10_drawn")
    check_px(img, "red stroke drawn", 650, 467, RED)

    d.click(884, 360)             # SAV
    check("canvas saved to FAT16", d.wait_serial("PAINT: saved 480x350 canvas to CANVAS.RAW"))
    d.quit()
    finish()


if __name__ == "__main__":
    main()
