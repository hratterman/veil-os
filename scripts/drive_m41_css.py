#!/usr/bin/env python3
"""M41 step 6 — CSS gap-closing, deterministic over loopback.

csstest.htm exercises:
  - calc()/clamp()/min()/max() length functions  -> CSS_CALC_OK
  - @media (prefers-color-scheme: dark) applied (Veil renders dark), while the
    light query is skipped -> CSS_DARK_OK, and .darkonly text is green not red
  - border-radius on a block background (rounded card) -> CSS_RADIUS_OK
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
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    click_link(d, "/web.htm")
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))

    m = len(d.serial())
    check("csstest link present", click_link(d, "/csstest.htm"))
    check("csstest rendered", d.wait_serial("BROWSER: rendered /csstest.htm", 20, m))

    check("calc()/clamp()/min()/max() evaluated", d.wait_serial("CSS_CALC_OK", 6, m))
    check("dark color-scheme media applied", d.wait_serial("CSS_DARK_OK", 6, m))
    check("border-radius background rendered", d.wait_serial("CSS_RADIUS_OK", 6, m))

    d.move(1000, 700)
    img = d.dump("m41_css")

    # The dark-mode-only paragraph must be GREEN (#40e090, applied), not the
    # light query's red (#ff0000, skipped). Scan its row band for a greenish px.
    def greenish():
        for y in range(PAGE_Y, PAGE_Y + 360, 2):
            for x in range(CONTENT_X + 20, CONTENT_X + 440, 3):
                r, g, b = img.at(x, y)
                if g > 150 and r < 130 and b < 170:
                    return True
        return False
    check("dark-mode text is green (light query skipped)", greenish())

    d.quit()
    finish()


main()
