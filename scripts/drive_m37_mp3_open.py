#!/usr/bin/env python3
"""M37 MP3: open TONE.MP3 from the file manager → it opens in the Audio app,
and pressing Play runs the from-scratch MP3 decoder (logs the decoded format).
(No virtio-sound device in the GUI harness, so playback itself is a no-op, but
the decode path is exercised end to end.)"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X, CONTENT_Y = 122, 84
ROW_H = 14
COL_W = 164


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("file manager launched", d.wait_serial("WM: launch 'files'", 5, m))
    check("files listed", d.wait_serial("FILES[0]:", 4, m))

    idx = None
    for line in d.serial().splitlines():
        mm = re.search(r"FILES\[(\d+)\]: TONE\.MP3", line)
        if mm:
            idx = int(mm.group(1))
    check("TONE.MP3 present on disk", idx is not None)

    def key(qcode):
        d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": qcode}}}])
        d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": qcode}}}])

    d.click(200, 71)
    for _ in range(idx):
        key("down")
    m = len(d.serial())
    key("ret")
    check("audio app opened for MP3", d.wait_serial("FILES: open TONE.MP3 in Audio", 6, m))

    # Click the PLAY button (content (105..195, 88..118) → screen ~(467..557, 412..442))
    # to trigger the MP3 decode + playback kernel task.
    m = len(d.serial())
    d.click(512, 427)
    check("MP3 playback triggered", d.wait_serial("AUDIO: play TONE.MP3", 5, m))

    d.quit()
    finish()


main()
