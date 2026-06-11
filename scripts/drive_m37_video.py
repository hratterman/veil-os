#!/usr/bin/env python3
"""M37 H.264: open QUAD.MP4 from the file manager and confirm the from-scratch
H.264 baseline decoder produces frames that render and play (frame advances).
Mirrors drive_m35_video.py but for the .mp4 (H.264) path."""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X, CONTENT_Y = 122, 84
ROW_H = 14
COL_W = 164


def content_palette(img, x0, y0, w, h):
    seen = set()
    for y in range(y0, y0 + h, 5):
        for x in range(x0, x0 + w, 5):
            seen.add(img.at(x, y))
    return seen


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("file manager launched", d.wait_serial("WM: launch 'files'", 5, m))
    check("files listed", d.wait_serial("FILES[0]:", 4, m))

    idx = None
    for line in d.serial().splitlines():
        mm = re.search(r"FILES\[(\d+)\]: QUAD\.MP4", line)
        if mm:
            idx = int(mm.group(1))
    check("QUAD.MP4 present on disk", idx is not None)

    # Focus the file manager (click its title bar) then arrow down to QUAD.MP4
    # and Enter — it may be scrolled below the fold.
    def key(qcode):
        d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": qcode}}}])
        d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": qcode}}}])

    d.click(200, 71)
    for _ in range(idx):
        key("down")
    m = len(d.serial())
    key("ret")
    check("video player opened", d.wait_serial("FILES: open QUAD.MP4 in Video player", 6, m))
    check("H.264 decoded frames", d.wait_serial("VIDEO: QUAD.MP4 -> H.264", 6, m))
    check("first frame 320x240", d.wait_serial("VIDEO: frame 0 320x240", 6, m))

    vx, vy = 202, 104
    img1 = d.dump("m37_video_a")
    time.sleep(0.8)
    img2 = d.dump("m37_video_b")
    p1 = content_palette(img1, vx + 30, vy + 30, 220, 150)
    p2 = content_palette(img2, vx + 30, vy + 30, 220, 150)
    check("H.264 frame renders (many colours)", len(p1) >= 4, f"{len(p1)} colours")
    check("video is playing (frame changed)", p1 != p2)

    d.quit()
    finish()


main()
