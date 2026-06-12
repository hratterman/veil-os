#!/usr/bin/env python3
"""M38 JS engine in the browser: navigate to a JS-rendered page (JSTEST.HTM —
the real henryratterman.com render code: shared.js + content.js + the inline
render() engine, all inlined) over loopback. Prove the browser runs the scripts
and the DOM gets populated (body text grows from ~empty to thousands of chars),
then renders many colours."""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

CONTENT_Y = 52
PAGE_X, PAGE_Y = 14, 74


def type_str(d, s):
    for ch in s:
        qcode = {"/": "slash", ".": "dot"}.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def palette(img, x0, y0, w, h):
    seen = set()
    for y in range(y0, y0 + h, 6):
        for x in range(x0, x0 + w, 6):
            seen.add(img.at(x, y))
    return seen


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    d.click(650, CONTENT_Y + 10)  # address bar
    type_str(d, "/jstest.htm")
    press(d, "ret")
    check("JS page rendered", d.wait_serial("BROWSER: rendered /jstest.htm", 25, m))
    check("scripts ran", d.wait_serial("BROWSER: ran 3 script(s)", 5, m))

    # The "body text now N chars" log proves the DOM was populated by JS.
    chars = None
    for line in d.serial().splitlines():
        mm = re.search(r"body text now (\d+) chars", line)
        if mm:
            chars = int(mm.group(1))
    check("JS populated the DOM (body text > 2000 chars)", chars is not None and chars > 2000,
          f"body text = {chars} chars")

    # Visual: the rendered page should have lots of content (many colours).
    time.sleep(0.4)
    img = d.dump("m38_js")
    pal = palette(img, PAGE_X + 10, PAGE_Y + 10, 980, 600)
    check("page renders content (many colours)", len(pal) >= 8, f"{len(pal)} colours")

    d.quit()
    finish()


main()
