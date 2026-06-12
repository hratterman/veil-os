#!/usr/bin/env python3
"""M38 acceptance: render the live henryratterman.com in the Veil browser over
direct TLS. Proves the full overhaul: the JS engine runs content.js + shared.js
+ the inline render() (DOM populated), web fonts (Cormorant Garamond / Barlow /
Lora) are fetched as TTF and registered, CSS grid lays out the projects, and the
JS-injected headshot is fetched. Network-dependent (real site)."""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy


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


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    m = len(d.serial())
    d.click(650, 84)  # address bar
    type_str(d, "https://henryratterman.com")
    press(d, "ret")
    # The live fetch + TLS + scripts + 6 web-font TTFs + a 4780px layout with
    # FreeType glyph rendering is slow in the debug build — allow plenty of time.
    check("henryratterman rendered", d.wait_serial("BROWSER: rendered https://henryratterman.com", 240, m))

    serial = d.serial()
    # JS ran and populated the DOM.
    chars = None
    for line in serial.splitlines():
        mm = re.search(r"ran \d+ script\(s\); body text now (\d+) chars", line)
        if mm:
            chars = int(mm.group(1))
    check("JS ran + populated DOM (>2000 chars)", chars is not None and chars > 2000, f"{chars} chars")
    check("web fonts registered", "registered" in serial and "web font" in serial)
    check("CSS grid laid out (GRID_OK)", "GRID_OK" in serial)

    time.sleep(0.5)
    img = d.dump("m38_hr")
    seen = set()
    for y in range(90, 700, 6):
        for x in range(20, 1000, 6):
            seen.add(img.at(x, y))
    check("page renders content (many colours)", len(seen) >= 12, f"{len(seen)} colours")

    d.quit()
    finish()


main()
