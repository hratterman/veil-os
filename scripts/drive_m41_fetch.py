#!/usr/bin/env python3
"""M41 Step 1: ES6 fetch() + async/await end-to-end. Navigate to a page whose
inline script does `const res = await fetch('/echo?q=hello'); const t = await
res.text();` and writes the result into a div. Verify the browser actually
performed the fetch over the HTTP stack and the awaited result populated the
DOM. Browser needs a NIC (loopback HTTP server)."""
import re
import sys

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
    d.click(650, CONTENT_Y + 32)  # address bar
    type_str(d, "/fetchte.htm")
    press(d, "ret")
    check("fetch page rendered", d.wait_serial("BROWSER: rendered /fetchte.htm", 25, m))

    # The inline async script must have invoked fetch() over the HTTP stack.
    check("fetch() called over the HTTP stack", d.wait_serial("BROWSER: fetch() -> /echo", 10, m))
    check("server served the /echo request", d.wait_serial("HTTP: /echo query=", 10, m))

    # The awaited result (console.log'd by the page) reaches serial via the
    # browser's "js: ... first: [console] ..." line.
    body_text = d.serial()
    check("awaited fetch result correct (status 200, ok, has body)",
          "FETCHED status=200 ok=true" in body_text and "hasSubmitted=true" in body_text,
          next((l for l in body_text.splitlines() if "FETCHED" in l), "no FETCHED line"))

    d.move(950, 650)
    d.dump("m41_fetch")
    d.quit()
    finish()


main()
