#!/usr/bin/env python3
"""M35 MJPEG video: open DEMO.MJP from the file manager and confirm it decodes
and plays (frames advance). No-NIC taskbar: files is idx 7 -> x=652. File
manager window (120,60), content origin (122,84), rows 14px, col width 164."""
import re
import sys

from guilib import Driver, check, finish

FILES_BTN = (652, 748)
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
    d.click(*FILES_BTN)
    check("file manager launched", d.wait_serial("WM: launch 'files'", 5, m))
    check("files listed", d.wait_serial("FILES[0]:", 4, m))

    # Find DEMO.MJP's row index from the FILES[i] log and click it.
    idx = None
    for line in d.serial().splitlines():
        mm = re.search(r"FILES\[(\d+)\]: DEMO\.MJP", line)
        if mm:
            idx = int(mm.group(1))
    check("DEMO.MJP present on disk", idx is not None)
    rows = (378 - 2 - 22) // ROW_H
    col, row = idx // rows, idx % rows
    sx = CONTENT_X + col * COL_W + 80
    sy = CONTENT_Y + row * ROW_H + 7

    m = len(d.serial())
    d.click(sx, sy)
    check("video player opened", d.wait_serial("FILES: open DEMO.MJP in Video player", 5, m))
    check("mjpeg split into frames", d.wait_serial("VIDEO: DEMO.MJP -> 48 frames", 4, m))
    check("first frame decoded", d.wait_serial("VIDEO: frame 0 192x144", 5, m))

    # Window content (video at ~200,80 size 360x300; content origin (202,104)).
    vx, vy = 202, 104
    img1 = d.dump("m35_video_a")
    # Let it play, then dump again — the frame must change (proof of playback).
    import time
    time.sleep(0.8)
    img2 = d.dump("m35_video_b")
    p1 = content_palette(img1, vx + 40, vy + 40, 200, 140)
    p2 = content_palette(img2, vx + 40, vy + 40, 200, 140)
    check("video frame renders (many colours)", len(p1) >= 20, f"{len(p1)} colours")
    check("video is playing (frame changed)", p1 != p2)

    d.quit()
    finish()


main()
