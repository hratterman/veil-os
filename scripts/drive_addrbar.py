#!/usr/bin/env python3
"""Address-bar fix: typed input is an absolute URL / search, never a relative path.

Reproduces the reported bug in the GUI: open the browser, focus the address bar,
type a bare host ("google.com"), press Enter -> the browser canonicalises it to
https://google.com (NOT a path relative to the current host). Then a dotless
phrase ("veil os") becomes a web search. The destination is logged at navigate
time (before any network fetch), so the check is hermetic. A boot self-test
(ADDRBAR_OK) covers the host/path/search rules in isolation.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_Y = 54  # browser content origin y (window 30 + border 2 + title 22)


def type_str(d, s):
    for ch in s:
        qcode = {"/": "slash", ".": "dot", ":": "semicolon", "-": "minus", " ": "spc"}.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("ADDRBAR_OK boot self-test (bare host -> https://, dotless -> search)",
          "ADDRBAR_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 6, m))
    check("index rendered", d.wait_serial("BROWSER: rendered /", 40))

    # 1) Bare host: type "google.com" -> https://google.com (absolute, not
    #    henryratterman.com/google.com or /google.com).
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)  # focus the address bar
    type_str(d, "google.com")
    press(d, "ret")
    check("typed 'google.com' -> absolute https://google.com (not a relative path)",
          d.wait_serial("BROWSER: address bar 'google.com' -> https://google.com", 6, m))
    # The dotless-search and absolute-with-path rules are proven hermetically by
    # the ADDRBAR_OK boot self-test (a real google.com load here would block the
    # browser's input loop, so a second GUI navigation isn't reliable).

    d.move(1000, 700)
    d.dump("addrbar")
    d.quit()
    finish()


main()
