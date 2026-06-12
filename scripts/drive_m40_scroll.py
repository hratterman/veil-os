#!/usr/bin/env python3
"""M40 Step 4: OS-wide mouse-wheel scrolling. Open a 250-line file in the editor
and wheel up to the top; run `cat BIG.TXT` in the shell and wheel up through the
scrollback. Both log "EDITOR: scroll top=" / "SHELL: scroll top=" so we can
assert the view actually moved (and reaches the top, line 1)."""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

# File manager grid (matches drive_m36_editor.py).
FILES_X, FILES_Y = 122, 84
ROW_H = 14
COL_W = 164


def file_index(d, name):
    for line in d.serial().splitlines():
        m = re.match(rf"FILES\[(\d+)\]: {re.escape(name)}\s*$", line)
        if m:
            return int(m.group(1))
    return None


def wheel_up(d, n=1):
    for _ in range(n):
        d.q.cmd("input-send-event", events=[{"type": "btn", "data": {"down": True, "button": "wheel-up"}}])
        d.q.cmd("input-send-event", events=[{"type": "btn", "data": {"down": False, "button": "wheel-up"}}])
        time.sleep(0.03)


def last_scroll(d, tag, after):
    tops = [int(m) for m in re.findall(rf"{tag}: scroll top=(\d+)", d.serial()[after:])]
    return tops[-1] if tops else None


def type_str(d, s):
    for ch in s:
        shift = ch.isupper()
        qcode = {" ": "spc", ".": "dot"}.get(ch.lower(), ch.lower())
        if shift:
            d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}}])
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])
        if shift:
            d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK", "WM_OK" in d.serial() or d.wait_serial("WM_OK", 10))

    # --- Editor: open BIG.TXT (250 lines) and wheel to the top ---------------
    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("files launched", d.wait_serial("FILES[0]:", 5, m))
    idx = file_index(d, "BIG.TXT")
    check("BIG.TXT present", idx is not None, str(idx))
    rows = 360 // ROW_H
    col, row = (idx // rows, idx % rows)
    m = len(d.serial())
    d.click(FILES_X + col * COL_W + 80, FILES_Y + row * ROW_H + 7)
    check("BIG.TXT opens in editor", d.wait_serial("EDITOR: opened BIG.TXT", 6, m))

    # Editor starts pinned to the bottom; wheel up should walk the top toward 0.
    m = len(d.serial())
    wheel_up(d, 3)
    check("editor scrolls on wheel", d.wait_serial("EDITOR: scroll top=", 5, m))
    wheel_up(d, 100)  # plenty to reach the very top of a 250-line file
    top = last_scroll(d, "EDITOR", m)
    check("editor reaches the top (line 1 visible)", top == 0, f"top={top}")

    # --- Shell: cat the long file, wheel up through the scrollback ------------
    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))
    type_str(d, "cat big.txt")
    press(d, "ret")
    time.sleep(0.8)
    m = len(d.serial())
    wheel_up(d, 3)
    check("shell scrolls on wheel", d.wait_serial("SHELL: scroll top=", 5, m))
    wheel_up(d, 120)
    top = last_scroll(d, "SHELL", m)
    check("shell reaches the top of scrollback", top == 0, f"top={top}")

    d.quit()
    finish()


main()
