#!/usr/bin/env python3
"""M35 Veil Breakout: launch from the shell, launch the ball, and confirm it
breaks bricks (scores). No-NIC: shell is taskbar idx 4 (x=418)."""
import sys

from guilib import Driver, check, finish, taskbar_xy

SHELL_BTN = (418, 748)


def keyev(d, key, down):
    d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))

    # `breakout` launches the game (and focuses it).
    m = len(d.serial())
    for ch in "breakout":
        keyev(d, ch, True)
        keyev(d, ch, False)
    keyev(d, "ret", True)
    keyev(d, "ret", False)
    check("breakout launched", d.wait_serial("BREAKOUT: new game", 5, m))

    # Focus the breakout window, launch the ball, wait for a brick break.
    d.click(500, 250)
    m = len(d.serial())
    keyev(d, "spc", True)
    keyev(d, "spc", False)
    check("ball breaks a brick (scores)", d.wait_serial("BREAKOUT: score", 14, m))

    d.move(1000, 700)
    d.dump("m35_breakout")
    d.quit()
    finish()


main()
