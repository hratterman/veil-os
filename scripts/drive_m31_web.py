#!/usr/bin/env python3
"""M31 site proof: the on-OS browser renders the new pages and the shared
nav links work (no 404s). Launch the browser, hop across several new pages
by clicking their nav links, and require each to render."""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

# Browser is taskbar idx 2 (NIC present): x = 70 + 2*78 + 36 = 262.
BROWSER_BTN = (70 + 2 * 78 + 36, 768 - 20)
WIN_X, WIN_Y = 510, 30
CONTENT_X = WIN_X + 2
PAGE_Y = WIN_Y + 2 + 22 + 20  # border + title + url bar
VIEW_H = 620 - 20


def boxes(serial, kind, after=0):
    pat = rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)"
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(pat, serial[after:])]


def cur_scroll(d):
    m = re.findall(r"BROWSER: scroll y=(\d+)", d.serial())
    return int(m[-1]) if m else 0


def goto(d, href):
    """Click the nav link to `href` and require it renders."""
    mark = len(d.serial())
    links = [b for b in boxes(d.serial(), "link") if b[0] == href]
    if not links:
        check(f"link {href} present", False, "not laid out")
        return
    _, x, y, w, h = links[-1]
    # nav is at the very top of every page (scroll 0); click it.
    d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2 - cur_scroll(d))
    check(f"navigated to {href}", d.wait_serial(f"BROWSER: rendered {href}", 20, mark))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    mark = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, mark))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # Hop across the new pages via their nav links.
    for href in ("/news.htm", "/wiki.htm", "/gallery.htm", "/ascii.htm",
                 "/tips.htm", "/changes.htm", "/about.htm", "/index.htm"):
        goto(d, href)

    check("no 404s while browsing", "BROWSER: 404" not in d.serial()
          and " 404 " not in d.serial())
    d.quit()
    finish()


if __name__ == "__main__":
    main()
