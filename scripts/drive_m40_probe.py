#!/usr/bin/env python3
"""M40 Step 6 probe: navigate to reddit.com and x.com, capture serial + a
screenshot of each so we can see what's broken before fixing. Not a pass/fail
gate — it prints diagnostics."""
import re
import sys
import time

from guilib import Driver, finish, taskbar_xy

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


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def visit(d, url, shot):
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)
    type_str(d, url)
    press(d, "ret")
    # wait for a render or a clear failure
    for _ in range(220):
        s = d.serial()[m:]
        if "BROWSER: rendered " in s or "error page" in s or "fetch failed" in s:
            break
        time.sleep(0.5)
    time.sleep(1.0)
    d.move(950, 650)
    d.dump(shot)
    tail = d.serial()[m:]
    print(f"\n===== {url} =====")
    for line in tail.splitlines():
        if re.search(r"rendered |error page|fetch failed|status |redirect|doc 480x|ran \d+ script|TLS|not html|too large|PANIC|decoded", line):
            print("  " + line[:160])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    d.click(*taskbar_xy(d, "browser"))
    d.wait_serial("BROWSER: rendered / -", 40)
    visit(d, "https://reddit.com", "m40_reddit")
    visit(d, "https://x.com", "m40_x")
    visit(d, "https://old.reddit.com", "m40_oldreddit")
    finish()


main()
