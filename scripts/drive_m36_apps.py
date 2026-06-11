#!/usr/bin/env python3
"""M36 calculator + settings apps."""
import sys

from guilib import Driver, check, finish, taskbar_xy


def key(d, qcode):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def shifted(d, qcode):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}}])
    key(d, qcode)
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK", "WM_OK" in d.serial())

    # Calculator: 7 * 8 = 56
    m = len(d.serial())
    d.click(*taskbar_xy(d, "calc"))
    check("calc launched", d.wait_serial("WM: launch 'calc'", 5, m))
    m = len(d.serial())
    key(d, "7")
    shifted(d, "8")  # '*'
    key(d, "8")
    key(d, "equal")
    check("7 * 8 = 56", d.wait_serial("CALC: 7 * 8 = 56", 5, m))

    # A second computation: 100 / 4 = 25
    m = len(d.serial())
    for k in ["1", "0", "0", "slash", "4", "equal"]:
        key(d, k)
    check("100 / 4 = 25", d.wait_serial("CALC: 100 / 4 = 25", 5, m))

    # Settings: launch, click the Sound page (sidebar item 1 at y~110+8+34=42).
    m = len(d.serial())
    d.click(*taskbar_xy(d, "settings"))
    check("settings launched", d.wait_serial("WM: launch 'settings'", 5, m))

    d.move(900, 650)
    d.dump("m36_apps")
    d.quit()
    finish()


main()
