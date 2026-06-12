#!/usr/bin/env python3
"""M41 step 7 — HTML5 Canvas 2D API.

Boot already proves the rasterizer (CANVAS_OK self-test). Here the browser loads
canvas.htm, whose inline <script> draws a 6-bar bar chart into a <canvas>
via getContext('2d') — fillRect bars, a stroked axis, fillText labels. We confirm
the canvas was drawn (BROWSER: 1 canvas(es) drawn by JS / CANVAS_PAGE_OK) and
that distinct bar colors actually appear in the rendered canvas box.
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
    check("CANVAS_OK boot self-test (rasterizer)", "CANVAS_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    click_link(d, "/web.htm")
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))

    m = len(d.serial())
    check("canvastest link present", click_link(d, "/canvas.htm"))
    check("canvastest rendered", d.wait_serial("BROWSER: rendered /canvas.htm", 20, m))
    check("canvas drawn by JS", d.wait_serial("canvas(es) drawn by JS", 6, m))
    check("canvas composited into page", d.wait_serial("CANVAS_PAGE_OK", 6, m))

    d.move(1000, 700)
    img = d.dump("m41_canvas")

    # Find the canvas box in document coords and sample it for distinct,
    # saturated bar colors (the chart uses 6 vivid fills on white).
    cv = [b for b in boxes(d.serial(), "img") if "__canvas" in b[0]]
    check("canvas box laid out", bool(cv))
    if cv:
        _, cx, cy, cw, ch = cv[-1]
        vivid = set()
        for yy in range(0, ch, 3):
            for xx in range(0, cw, 3):
                sx = CONTENT_X + cx + xx
                sy = PAGE_Y + cy + yy
                if sx < CONTENT_X or sx > CONTENT_X + 470:
                    continue
                r, g, b = img.at(sx, sy)
                mx, mn = max(r, g, b), min(r, g, b)
                # saturated (not white/gray/black): big channel spread, bright
                if mx - mn > 70 and mx > 110:
                    vivid.add((r // 48, g // 48, b // 48))
        check(f"multiple distinct bar colors rendered ({len(vivid)})", len(vivid) >= 4)

    d.quit()
    finish()


main()
