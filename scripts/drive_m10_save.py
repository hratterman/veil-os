#!/usr/bin/env python3
"""M10 boot 1: draw a red stroke in Paint and SAVE it to the filesystem."""
import sys
from guilib import Driver, check, check_px, finish

RED = (224, 48, 48)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("M10_OK at boot", "M10_OK" in d.serial())

    d.click(700, 340)             # raise the paint window
    d.click(526, 368)             # palette: red
    d.drag(550, 450, 750, 500, steps=6)
    img = d.dump("m10_drawn")
    check_px(img, "red stroke drawn", 650, 475, RED)

    d.click(884, 368)             # SAV
    check("canvas saved to FAT16", d.wait_serial("PAINT: saved 480x350 canvas to CANVAS.RAW"))
    d.quit()
    finish()


if __name__ == "__main__":
    main()
