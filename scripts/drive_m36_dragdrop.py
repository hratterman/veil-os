#!/usr/bin/env python3
"""M36 drag and drop: drag a JPEG out of the file manager -> it opens."""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X, CONTENT_Y, ROW_H = 122, 84, 14


def file_index(d, name):
    for line in d.serial().splitlines():
        m = re.match(rf"FILES\[(\d+)\]: {re.escape(name)}\s*$", line)
        if m:
            return int(m.group(1))
    return None


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("files launched", d.wait_serial("FILES[0]:", 4, m))

    idx = file_index(d, "DOG.JPG")
    check("DOG.JPG present", idx is not None, str(idx))
    fy = CONTENT_Y + idx * ROW_H + 7
    # Drag the file from its row out to the empty desktop on the right.
    m = len(d.serial())
    d.drag(200, fy, 700, 420, steps=12)
    check("file dropped", d.wait_serial("WM: dropped 'DOG.JPG'", 5, m))
    check("dropped file opens", d.wait_serial("FILES: open DOG.JPG", 5, m))

    d.move(950, 650)
    d.dump("m36_dragdrop")
    d.quit()
    finish()


main()
