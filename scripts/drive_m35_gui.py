#!/usr/bin/env python3
"""M35 GUI overhaul screenshot: open a few apps and capture the modern dark
look (no-NIC taskbar: edit=106, clock=184, snake=886)."""
import sys

from guilib import Driver, check, finish, taskbar_xy


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    for app in ["edit", "clock", "snake"]:
        d.click(*taskbar_xy(d, app))
    d.move(1000, 700)
    img = d.dump("m35_gui")
    # The title bars must NOT be the old chunky blue (~rgb(48,96,192)).
    # Sample several window title rows; none should be that blue.
    blue = sum(1 for y in range(60, 120)
               for x in range(100, 900)
               if img.at(x, y) == (48, 96, 192))
    check("no chunky blue title bars", blue < 50, f"{blue} chunky-blue px")
    # Desktop background should be near-black, not teal.
    bg = img.at(700, 600)
    check("near-black desktop background", max(bg) < 40, f"bg rgb{bg}")
    d.quit()
    finish()


main()
