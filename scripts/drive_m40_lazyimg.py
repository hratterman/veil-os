#!/usr/bin/env python3
"""M40 Step 7: viewport-aware image loading. A 10-image page (~3800px tall, each
img a distinct slot via ?N) should decode only the images near the top at load,
defer the rest as placeholders, and lazily decode them as you scroll down.
"""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

CONTENT_Y = 52


def type_str(d, s):
    smap = {"/": "slash", ".": "dot", "?": "slash"}
    for ch in s:
        if ch == "?":
            d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}}])
            for down in (True, False):
                d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": "slash"}}}])
            d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}}])
            continue
        qcode = smap.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key, n=1):
    for _ in range(n):
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def decoded_set(serial):
    return set(re.findall(r"BROWSER: (?:lazy-)?decoded (/logo\.png\?\d+)", serial))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)
    type_str(d, "/imgs.htm")
    press(d, "ret")
    check("image page rendered", d.wait_serial("BROWSER: rendered /imgs.htm", 25, m))

    # At load: only the near-viewport images decoded; the rest deferred.
    load_serial = d.serial()[m:]
    at_load = decoded_set(load_serial)
    check("some images deferred at load (not all 10 fetched)",
          0 < len(at_load) < 10, f"decoded at load = {len(at_load)}: {sorted(at_load)}")
    check("browser logged deferred images", "image(s) deferred" in load_serial)

    # Scroll to the bottom: deferred images lazily decode as they near the view.
    m2 = len(d.serial())
    press(d, "pgdn", 20)
    time.sleep(1.0)
    after = decoded_set(d.serial())
    check("more images decoded after scrolling", len(after) > len(at_load),
          f"after scroll = {len(after)} (was {len(at_load)})")
    check("lazy-load fired on scroll", "lazy-decoded" in d.serial()[m2:])

    d.move(950, 650)
    d.dump("m40_lazyimg")
    d.quit()
    finish()


main()
