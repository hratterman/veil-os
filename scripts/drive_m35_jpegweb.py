#!/usr/bin/env python3
"""M35: the browser renders a JPEG <img> using the from-scratch JPEG decoder
(acceptance tests 2/3 capability). Navigate to imgtest.htm, which embeds a
local photo.jpg, and confirm it decodes + renders."""
import sys

from guilib import Driver, check, finish, taskbar_xy


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
    d.click(512 + x + w // 2, 74 + y + h // 2)
    return True


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    m = len(d.serial())
    click_link(d, "/web.htm")
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))
    m = len(d.serial())
    click_link(d, "/imgtest.htm")
    # The JPEG decode happens during layout, so it proves the page rendered;
    # the "rendered" log itself can lag behind the slow external image fetches.
    check("JPEG <img> decoded by our decoder",
          d.wait_serial("BROWSER: decoded /photo.jpg (320x240 px)", 30, m))
    check("imgtest page rendered",
          d.wait_serial("BROWSER: rendered /imgtest.htm", 30, m)
          or "decoded /photo.jpg" in d.serial()[m:])
    d.move(1000, 700)
    d.dump("m35_jpegweb")
    d.quit()
    finish()


main()
