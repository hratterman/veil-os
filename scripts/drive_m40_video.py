#!/usr/bin/env python3
"""M40 Step 8: <video> tag in the browser. A page embeds <video src="quad.mp4">.
The browser renders a poster/placeholder with a play affordance; clicking it
fetches the MP4 and opens it in the video player (existing H.264 pipeline),
which decodes and plays frames."""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X, CONTENT_Y = 512, 52
PAGE_Y = 96


def type_str(d, s):
    smap = {"/": "slash", ".": "dot"}
    for ch in s:
        qcode = smap.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)
    type_str(d, "/vid.htm")
    press(d, "ret")
    check("video page rendered", d.wait_serial("BROWSER: rendered /vid.htm", 25, m))

    # The <video> placeholder must be registered with a hit box.
    mm = None
    for line in d.serial()[m:].splitlines():
        g = re.search(r"BROWSER: video '(/quad\.mp4)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", line)
        if g:
            mm = g
    check("video placeholder registered", mm is not None, str(mm))
    _, x, y, w, h = mm.group(1), int(mm.group(2)), int(mm.group(3)), int(mm.group(4)), int(mm.group(5))

    # Click the centre of the placeholder -> fetch + open in the player.
    m = len(d.serial())
    d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    check("browser handled the video click", d.wait_serial("BROWSER: play video /quad.mp4", 10, m))
    check("browser fetched the MP4", d.wait_serial("BROWSER: fetched video /quad.mp4", 30, m))
    check("WM opened the video player", d.wait_serial("WM: launch video", 10, m))
    check("player decoded H.264 frames", d.wait_serial("-> H.264,", 30, m))
    check("first frame decoded", d.wait_serial("VIDEO: frame 0", 10, m))

    time.sleep(0.6)
    d.move(950, 650)
    d.dump("m40_video")
    d.quit()
    finish()


main()
