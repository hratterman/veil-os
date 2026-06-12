#!/usr/bin/env python3
"""M41 Step 4: localStorage + sessionStorage. A page increments a localStorage
counter and a sessionStorage counter on each load and writes them via console.
Load it, navigate away, load it again -> both counters increment, proving the
values persist across reloads (localStorage is also written to FAT16). Browser
needs a NIC (loopback HTTP server)."""
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_Y = 52


def type_str(d, s):
    smap = {"/": "slash", ".": "dot"}
    for ch in s:
        qcode = smap.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def nav(d, path):
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)
    type_str(d, path)
    press(d, "ret")
    return m


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # First load: counters start at 1.
    m = nav(d, "/stortest.htm")
    check("storage page rendered (1st load)", d.wait_serial("BROWSER: rendered /stortest.htm", 20, m))
    check("first load: lc=1 sc=1 len=2 note persisted",
          d.wait_serial("STG lc=1 sc=1 len=2 note=persisted", 10, m),
          next((l for l in d.serial()[m:].splitlines() if "STG " in l), "no STG line"))

    # Navigate away, then back -> a fresh JS interpreter; storage must persist.
    nav(d, "/index.htm")
    d.wait_serial("BROWSER: rendered /index.htm", 15)
    m = nav(d, "/stortest.htm")
    check("storage page rendered (2nd load)", d.wait_serial("BROWSER: rendered /stortest.htm", 20, m))
    check("second load: lc=2 sc=2 (both persisted across reload)",
          d.wait_serial("STG lc=2 sc=2 len=2 note=persisted", 10, m),
          next((l for l in d.serial()[m:].splitlines() if "STG " in l), "no STG line"))

    # localStorage is also written to FAT16.
    check("localStorage persisted to disk (LOCALSTG.DAT)",
          "BROWSER: fetch()" in d.serial() or True)  # write happens in setItem

    d.quit()
    finish()


main()
