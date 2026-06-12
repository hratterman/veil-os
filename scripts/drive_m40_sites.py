#!/usr/bin/env python3
"""M40 Step 6: Reddit + Twitter/X smoke test.

- reddit.com is a blank React SPA without its JS bundle, so the browser now
  transparently routes reddit.com -> old.reddit.com (server-rendered HTML) and
  a real post list (hundreds of links) shows.
- x.com serves a server-rendered "JavaScript is not available" page (the real
  X fallback) that the browser renders cleanly — it looks like the actual site.
"""
import re
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

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


def goto(d, url):
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)
    type_str(d, url)
    press(d, "ret")
    return m


def render_stats(serial, host):
    items = links = doc_h = None
    for line in serial.splitlines():
        mm = re.search(rf"rendered {re.escape(host)}\S* - (\d+) items, (\d+) links, doc 480x(\d+)", line)
        if mm:
            items, links, doc_h = int(mm[1]), int(mm[2]), int(mm[3])
    return items, links, doc_h


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("BROWSER: rendered / -", 40))

    # --- Reddit: reddit.com must route to old.reddit.com and show a post list --
    m = goto(d, "https://reddit.com")
    check("reddit.com routed to old.reddit.com (compat shim)",
          d.wait_serial("https://old.reddit.com", 60, m))
    check("old.reddit front page rendered",
          d.wait_serial("BROWSER: rendered https://old.reddit.com", 240, m))
    items, links, doc_h = render_stats(d.serial()[m:], "https://old.reddit.com")
    check("reddit shows a readable post list (100+ links)",
          links is not None and links > 100, f"items={items} links={links} doc_h={doc_h}")
    d.dump("m40_sites_reddit")

    # --- X/Twitter: renders the real server-side noscript page ----------------
    m = goto(d, "https://x.com")
    check("x.com rendered", d.wait_serial("BROWSER: rendered https://x.com", 120, m))
    items, links, doc_h = render_stats(d.serial()[m:], "https://x.com")
    check("x.com renders site content (items + links)",
          items is not None and items > 10 and (links or 0) >= 3,
          f"items={items} links={links} doc_h={doc_h}")
    d.dump("m40_sites_x")

    d.quit()
    finish()


main()
