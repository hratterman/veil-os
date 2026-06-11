#!/usr/bin/env python3
"""M36 window management: maximize (double-click title), restore, resize (drag
corner), minimize (button) + restore from taskbar pill."""
import sys

from guilib import Driver, check, finish, taskbar_xy

# Editor opens at (40,40) content 420x300; BORDER 2, TITLE_H 22.
# frame right/bottom = (40+424, 40+324) = (464, 364). Title bar y ~ 51.
WX, WY, CW, CH = 40, 40, 420, 300
TITLE_Y = WY + 2 + 11


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "edit"))
    check("editor launched", d.wait_serial("WM: launch 'edit'", 5, m))

    # Double-click the title bar -> maximize.
    m = len(d.serial())
    d.click(200, TITLE_Y)
    d.click(200, TITLE_Y)
    check("double-click maximizes", d.wait_serial("WM: maximized 'edit'", 5, m))

    # Double-click again -> restore. Maximized title bar is now at the top (y~13).
    m = len(d.serial())
    d.click(200, 13)
    d.click(200, 13)
    check("double-click restores", d.wait_serial("WM: restored 'edit'", 5, m))

    # Resize: drag the bottom-right corner outward.
    m = len(d.serial())
    d.drag(WX + CW + 4, WY + CH + 4 + 22, 700, 560, steps=10)
    check("corner drag resizes", d.wait_serial("WM: resized 'edit'", 5, m))

    d.move(1000, 700)
    d.dump("m36_winmgmt")

    # Minimize via the min button (3rd from right). After resize, recompute the
    # right edge from the resized geometry is hard; re-open at default first.
    # Instead test minimize on a freshly raised window: launch clock.
    m = len(d.serial())
    d.click(*taskbar_xy(d, "clock"))
    d.wait_serial("WM: launch 'clock'", 5, m)
    # clock at (700,36) content 260x260; min button center:
    # tx=702, cw=260 -> min center = 702+260-60+10 = 912, y=36+2+11=49
    m = len(d.serial())
    d.click(912, 49)
    check("minimize button", d.wait_serial("WM: minimized 'clock'", 5, m))

    # Restore from taskbar pill.
    m = len(d.serial())
    d.click(*taskbar_xy(d, "clock"))
    check("restore from taskbar", d.wait_serial("restored 'clock' from taskbar", 5, m))

    d.quit()
    finish()


main()
