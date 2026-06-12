#!/usr/bin/env python3
"""M41 step 8 — IndexedDB (basic).

Boot proves the API end to end (IDB_OK self-test: open/createObjectStore/put/
get/getAll with structured records). Here the browser loads idbtest.htm whose
script stores a visit counter in IndexedDB; navigating away and back (a fresh
interpreter) reads the persisted value incremented — proving cross-reload
persistence through the localStorage->FAT16 backing store. The page console.logs
`IDB_PAGE visits=N`, which the browser surfaces on serial.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X = 512
PAGE_Y = 54 + 42


def boxes(s, kind):
    import re
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(
                rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]


def click_link(d, href_sub):
    lk = [b for b in boxes(d.serial(), "link") if href_sub in b[0]]
    if not lk:
        return False
    _, x, y, w, h = lk[-1]
    d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    return True


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("IDB_OK boot self-test (structured round-trip)", "IDB_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    click_link(d, "/web.htm")
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))

    # First visit: count becomes 1.
    m = len(d.serial())
    check("idbtest link present", click_link(d, "/idbtest.htm"))
    check("idbtest rendered", d.wait_serial("BROWSER: rendered /idbtest.htm", 20, m))
    check("IndexedDB stored visit 1", d.wait_serial("IDB_PAGE visits=1", 6, m))

    # Navigate away and back: a fresh interpreter reads the persisted count -> 2.
    m = len(d.serial())
    click_link(d, "/web.htm")
    check("back to web", d.wait_serial("BROWSER: rendered /web.htm", 20, m))
    m = len(d.serial())
    click_link(d, "/idbtest.htm")
    check("idbtest re-rendered", d.wait_serial("BROWSER: rendered /idbtest.htm", 20, m))
    check("IndexedDB persisted across reload (visit 2)", d.wait_serial("IDB_PAGE visits=2", 6, m))

    d.move(1000, 700)
    d.dump("m41_idb")
    d.quit()
    finish()


main()
