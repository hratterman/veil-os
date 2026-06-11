#!/usr/bin/env python3
"""M36 browser find-in-page (Ctrl+F)."""
import sys

from guilib import Driver, check, finish, taskbar_xy


def key(d, qcode):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def ctrl(d, qcode):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "ctrl"}}}])
    key(d, qcode)
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "ctrl"}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # Ctrl+F, type "veil" -> matches on the page.
    m = len(d.serial())
    ctrl(d, "f")
    for ch in "veil":
        key(d, ch)
    check("find finds matches", d.wait_serial("BROWSER: find 'veil' ->", 5, m))
    # The last find line should report a non-zero match count.
    finds = [l for l in d.serial().splitlines() if "BROWSER: find 'veil' ->" in l]
    last = finds[-1] if finds else ""
    n = int(last.split("->")[1].split("matches")[0].strip()) if "->" in last else 0
    check("at least one match", n >= 1, f"{n} matches")

    # Enter advances to the next match.
    m = len(d.serial())
    key(d, "ret")

    d.move(950, 300)
    d.dump("m36_find")
    d.quit()
    finish()


main()
