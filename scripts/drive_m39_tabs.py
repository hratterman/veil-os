#!/usr/bin/env python3
"""M39 browser UX: tabs, per-tab back/forward, zoom, Ctrl+click new tab.
Loopback (deterministic): open the index, Ctrl+click a link to open it in a new
tab, switch back to tab 0, then exercise back/forward and zoom. Proven on the
serial breadcrumbs the browser logs for each action."""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

# browser window at (510,30); content origin = (+2 border, +22 title +2) = (512,54)
OX, OY = 512, 54
CHROME = 42  # tab strip (22) + address bar (20)


def key(d, qcode, mods=()):
    for m in mods:
        d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": m}}}])
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])
    for m in mods:
        d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": m}}}])


def ctrl_click(d, x, y):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "ctrl"}}}])
    d.click(x, y)
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "ctrl"}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # Find a link on the index page (logged in document coords).
    link = None
    for line in d.serial().splitlines():
        mm = re.search(r"BROWSER: link '(/[a-z0-9.]+)' at \((\d+), (\d+)\) (\d+)x(\d+)", line)
        if mm and int(mm.group(3)) < 500:  # near the top, visible without scrolling
            link = (mm.group(1), int(mm.group(2)), int(mm.group(3)), int(mm.group(4)), int(mm.group(5)))
            break
    check("found a link on the index", link is not None, str(link))
    href, lx, ly, lw, lh = link
    sx, sy = OX + lx + lw // 2, OY + CHROME + ly + lh // 2

    # Ctrl+click the link -> opens in a NEW tab.
    m = len(d.serial())
    ctrl_click(d, sx, sy)
    check("ctrl+click opened a new tab", d.wait_serial("BROWSER: ctrl+click -> new tab", 6, m))
    check("new tab navigated", d.wait_serial("BROWSER: new tab -> ", 6, m))

    # Switch back to tab 0 by clicking the first tab cell in the strip.
    m = len(d.serial())
    d.click(OX + 30, OY + 10)  # tab 0 cell
    check("switched back to tab 0", d.wait_serial("BROWSER: switch to tab 0", 8, m))

    # Switch to tab 1 again.
    m = len(d.serial())
    d.click(OX + 180, OY + 10)  # tab 1 cell
    check("switched to tab 1", d.wait_serial("BROWSER: switch to tab 1", 8, m))

    # In tab 1, navigate so there's back history, then test back + forward.
    m = len(d.serial())
    d.click(650, OY + 32)  # address bar (in the address-bar row)
    for ch in "/news.htm":
        qc = {"/": "slash", ".": "dot"}.get(ch, ch)
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qc}}}])
    key(d, "ret")
    check("navigated in tab 1", d.wait_serial("BROWSER: rendered /news.htm", 15, m))

    m = len(d.serial())
    d.click(OX + 8, OY + 32)  # back button '<'
    check("back works", d.wait_serial("BROWSER: back to", 10, m))
    m = len(d.serial())
    d.click(OX + 26, OY + 32)  # forward button '>'
    check("forward works", d.wait_serial("BROWSER: forward to", 10, m))

    # Zoom in / reset.
    m = len(d.serial())
    key(d, "equal", mods=["ctrl"])
    check("zoom in", d.wait_serial("BROWSER: zoom 110%", 10, m))
    m = len(d.serial())
    key(d, "0", mods=["ctrl"])
    check("zoom reset", d.wait_serial("BROWSER: zoom 100%", 10, m))

    d.quit()
    finish()


main()
