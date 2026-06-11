#!/usr/bin/env python3
"""M35 direct kernel TCP: navigate to http://example.com via the address bar.
The browser fetches it over the kernel's own TCP/IP stack (DIRECT_HTTP_OK), not
the host proxy. Browser is taskbar idx 2 (x=262); address bar at content y~62."""
import sys

from guilib import Driver, check, finish

BROWSER_BTN = (262, 748)
CONTENT_X, CONTENT_Y = 512, 54


def keyev(d, key, down):
    d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def type_str(d, s):
    for ch in s:
        if ch == ":":
            keys = [("shift", True), ("semicolon", True), ("semicolon", False), ("shift", False)]
        elif ch == "/":
            keys = [("slash", True), ("slash", False)]
        elif ch == ".":
            keys = [("dot", True), ("dot", False)]
        else:
            keys = [(ch.lower(), True), (ch.lower(), False)]
        for q, down in keys:
            keyev(d, q, down)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*BROWSER_BTN)
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # Address bar -> http://example.com over kernel TCP.
    m = len(d.serial())
    d.click(650, CONTENT_Y + 8)
    type_str(d, "http://example.com")
    keyev(d, "ret", True)
    keyev(d, "ret", False)

    check("connected over kernel TCP (DNS + connect)",
          d.wait_serial("BROWSER: direct TCP example.com", 30, m))
    check("fetched directly, no host proxy (DIRECT_HTTP_OK)",
          d.wait_serial("DIRECT_HTTP_OK", 30, m))
    check("page rendered", d.wait_serial("BROWSER: rendered http://example.com", 30, m))

    d.move(1000, 700)
    d.dump("m35_net")
    d.quit()
    finish()


main()
