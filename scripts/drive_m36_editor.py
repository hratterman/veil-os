#!/usr/bin/env python3
"""M36 editor upgrade: open a .rs file -> line numbers + syntax highlighting."""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X, CONTENT_Y = 122, 84
ROW_H = 14
COL_W = 164


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

    idx = file_index(d, "DEMO.RS")
    check("DEMO.RS present", idx is not None, str(idx))
    # 2-column layout: rows = ch/ROW_H. Files window content height ~ 360.
    rows = 360 // ROW_H
    col, row = (0, idx) if idx < rows else (1, idx - rows)
    m = len(d.serial())
    d.click(CONTENT_X + col * COL_W + 80, CONTENT_Y + row * ROW_H + 7)
    check("DEMO.RS opens in editor", d.wait_serial("FILES: open DEMO.RS in Editor", 5, m))

    d.move(950, 650)
    d.dump("m36_editor")
    d.quit()
    finish()


main()
