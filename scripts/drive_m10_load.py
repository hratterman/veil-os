#!/usr/bin/env python3
"""M10 boot 2 (fresh power-on, same disk): LOAD the canvas and verify the
stroke drawn last boot is back on screen — the spec's reboot criterion."""
import sys
from guilib import Driver, check, check_px, finish

RED = (224, 48, 48)
WHITE = (255, 255, 255)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    log = d.serial()
    check("CANVAS.RAW listed at boot", "FS_LS: CANVAS.RAW" in log)

    d.click(700, 340)             # raise the paint window
    img = d.dump("m10_before_load")
    check_px(img, "canvas empty before load", 650, 475, WHITE)

    d.click(836, 368)             # LOD
    check("canvas loaded from FAT16", d.wait_serial("PAINT: loaded 480x350 canvas"))
    img = d.dump("m10_loaded")
    check_px(img, "stroke from previous boot restored", 650, 475, RED)
    d.quit()
    finish()


if __name__ == "__main__":
    main()
