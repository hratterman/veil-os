#!/usr/bin/env python3
"""M40 Step 5: henryratterman.com full navigation. The page is ~10500px tall;
before the band rasterizer it was clipped to ~1333px (could not scroll past the
hero). Now it lays out full height and rasterizes a moving band as you scroll,
so the whole page is reachable. This drives PageDown to the bottom and proves
the band re-rasterizes deep into the document (and the footer links exist)."""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

CONTENT_Y = 52


def type_str(d, s):
    smap = {"/": "slash", ".": "dot", ":": "semicolon", "-": "minus"}
    for ch in s:
        qcode = smap.get(ch, ch.lower())
        shift = ch == ":"
        if shift:
            d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}}])
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])
        if shift:
            d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}}])


def press(d, key, n=1):
    for _ in range(n):
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)  # address bar
    type_str(d, "https://henryratterman.com")
    press(d, "ret")
    check("henryratterman rendered",
          d.wait_serial("BROWSER: rendered https://henryratterman.com", 240, m))

    # The full page now lays out tall (was clipped to ~1333px before banding).
    doc_h = None
    for line in d.serial().splitlines():
        mm = re.search(r"rendered https://henryratterman\.com - .* doc 480x(\d+)", line)
        if mm:
            doc_h = int(mm.group(1))
    check("page lays out full height (>8000px)", doc_h is not None and doc_h > 8000, f"doc_h={doc_h}")

    # Footer / project links deep in the document exist (navigable targets).
    links = d.serial()
    check("deep project links present (arduous.io / uses)",
          "https://arduous.io" in links and "/uses/" in links)

    # Scroll all the way down with PageDown; the band must re-rasterize deep into
    # the document (proving lower sections are actually drawn, not clipped).
    m = len(d.serial())
    press(d, "pgdn", 45)
    time.sleep(0.5)
    tops = [int(t) for t in re.findall(r"band rasterized top=(\d+)", d.serial()[m:])]
    maxtop = max(tops) if tops else 0
    check("band re-rasterizes deep into the page (top>6000)", maxtop > 6000, f"max band top={maxtop}")

    # And the bottom of the page actually paints content (not blank).
    img = d.dump("m40_hrnav_bottom")
    seen = set()
    for y in range(CONTENT_Y + 60, 700, 6):
        for x in range(20, 470, 6):
            seen.add(img.at(x, y))
    check("bottom of page renders content (many colours)", len(seen) >= 8, f"{len(seen)} colours")

    d.quit()
    finish()


main()
