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

    # Launch Paint from the taskbar (idx 3); window clamps 8px up.
    mark = len(d.serial())
    d.click(340, 768 - 20)
    check("paint launched", d.wait_serial("WM: launch 'paint'", 5, mark))

    img = d.dump("m10_before_load")
    check_px(img, "canvas empty before load", 650, 467, WHITE)

    d.click(836, 360)             # LOD
    check("canvas loaded from FAT16", d.wait_serial("PAINT: loaded 480x350 canvas"))
    img = d.dump("m10_loaded")
    check_px(img, "stroke from previous boot restored", 650, 467, RED)
    d.quit()
    finish()


if __name__ == "__main__":
    main()
