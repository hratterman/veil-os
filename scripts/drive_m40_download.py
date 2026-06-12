#!/usr/bin/env python3
"""M40 Step 3: file downloads. Navigate to a page with a link to a PDF, click
it -> the browser fetches a non-renderable content type, saves it to the FAT16
disk (SAMPLE.PDF), and shows a "Downloaded ..." toast. Then open the file
manager and confirm SAMPLE.PDF is listed.

Browser needs a NIC (loopback HTTP server). Content origin (512,96)."""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X, CONTENT_Y = 512, 52
PAGE_Y = 96


def type_str(d, s):
    for ch in s:
        qcode = {"/": "slash", ".": "dot"}.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def links(s):
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(
                r"BROWSER: link '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # Go to the downloads page.
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)  # address bar
    type_str(d, "/downld.htm")
    press(d, "ret")
    check("downloads page rendered", d.wait_serial("BROWSER: rendered /downld.htm", 20, m))

    # Click the PDF link.
    lk = [b for b in links(d.serial()[m:]) if "sample.pdf" in b[0]]
    check("pdf link present", bool(lk))
    _, x, y, w, h = lk[-1]
    m = len(d.serial())
    d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)

    check("browser saved the download", d.wait_serial("BROWSER: downloaded /sample.pdf", 15, m))
    check("download named SAMPLE.PDF", d.wait_serial("-> SAMPLE.PDF", 5, m))
    check("toast shown", d.wait_serial("NOTIFY: Downloaded SAMPLE.PDF", 10, m))

    # Open the file manager; SAMPLE.PDF should be in the listing.
    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("files app launched", d.wait_serial("FILES:", 8, m))
    listed = any("SAMPLE.PDF" in l for l in d.serial()[m:].splitlines() if "FILES[" in l)
    check("SAMPLE.PDF appears in the file manager", listed)

    d.quit()
    finish()


main()
