"""M16 proof: drive the on-OS browser end to end.

The kernel opens the browser window on boot (desktop.rs) pointed at "/",
fetched from its own HTTP server over its own TCP stack via loopback.
This script:
  1. waits for the index render, pixel-checks the page (body bg from the
     stylesheet, the decoded PNG logo, the link underline color),
  2. clicks the '/page2.htm' link through QMP tablet injection and
     requires M16_OK + the page2 render,
  3. clicks 'back home' on page2 and requires the index render,
  4. exercises scrolling if the document is taller than the viewport.

Positions are not hardcoded: the kernel logs document-space link/img
boxes ("BROWSER: link '...' at (x, y) WxH") and this script maps them to
screen space through the known window geometry.
"""
import re
import sys

from guilib import Driver, check, check_px, finish, taskbar_xy

# Window geometry (desktop.rs / wm.rs): browser at (510, 30), content
# 480x620, BORDER=2, TITLE_H=22, browser URL bar TOPBAR=20.
WIN_X, WIN_Y = 510, 30
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22
PAGE_Y = CONTENT_Y + 20          # first document row on screen
VIEW_W, VIEW_H = 480, 620 - 20   # visible document area

BODY_BG = (0x14, 0x18, 0x1C)     # style.css body background-color
LINK_RGB = (0xE0, 0xA0, 0x40)    # style.css a color


def logo_pixel(x, y):
    """Mirror of mksite.py's generated-logo pixel function."""
    in_v = abs(x - 32) < (y - 8) // 2 + 2 and 8 <= y < 52 and abs(x - 32) > (y - 8) // 2 - 6
    if in_v:
        return (224, 160, 64)
    return (24 + x // 3, 40 + y // 2, min(120 + (x + y) // 2, 255))


def boxes(serial, kind, after=0):
    """Parse 'BROWSER: {kind} '<name>' at (x, y) WxH' lines."""
    pat = rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)"
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(pat, serial[after:])]


def rendered(serial, path, after=0):
    m = re.search(rf"BROWSER: rendered {re.escape(path)} - (\d+) items, (\d+) links, doc (\d+)x(\d+)",
                  serial[after:])
    return m and (int(m[1]), int(m[2]), int(m[3]), int(m[4]))


def cur_scroll(d):
    """The browser's actual current scroll offset (it logs every change,
    and clamps page-downs to doc_h - view_h — which the caller can't
    predict, so read it rather than bookkeep it)."""
    m = re.findall(r"BROWSER: scroll y=(\d+)", d.serial())
    return int(m[-1]) if m else 0


def scroll_into_view(d, y, h):
    """Page-down until document rows [y, y+h) are within the viewport;
    return the resulting actual scroll offset."""
    for _ in range(12):
        if y + h <= cur_scroll(d) + VIEW_H:
            break
        smark = len(d.serial())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down,
                     "key": {"type": "qcode", "data": "pgdn"}}}])
        d.wait_serial("BROWSER: scroll y=", 5, smark)
    return cur_scroll(d)


def main():
    qmp, serial_path, shots = sys.argv[1], sys.argv[2], sys.argv[3]
    d = Driver(qmp, serial_path, shots)

    # --- 1. launch the browser, index page renders ---------------------
    # UX overhaul: nothing opens at boot. Browser is taskbar idx 2 (NIC
    # present, so Chat is in the bar): x = 70 + 2*78 + 36 = 262, y bottom.
    mark = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, mark))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", timeout=40))
    log = d.serial()
    info = rendered(log, "/")
    check("index render stats parsed", bool(info), str(info))
    img = d.dump("m16_index")

    # body background (from the fetched stylesheet) in an empty page area
    check_px(img, "index: stylesheet body bg", CONTENT_X + 470, PAGE_Y + 6, BODY_BG)

    # the decoded PNG logo, sampled where the generator says a pixel is
    logos = [b for b in boxes(log, "img") if b[0] == "/logo.png"]
    check("logo placed", len(logos) == 1, str(logos))
    if logos:
        _, lx, ly, lw, lh = logos[0]
        sx, sy = CONTENT_X + lx + 10, PAGE_Y + ly + 10
        if ly + 10 < VIEW_H:
            check_px(img, "index: decoded logo pixel", sx, sy, logo_pixel(10, 10))

    # the page2 link: underline row must be solid link-color
    links = [b for b in boxes(log, "link") if b[0] == "/page2.htm"]
    check("page2 link laid out", len(links) == 1, str(links))
    if not links:
        finish()
    _, x, y, w, h = links[0]

    # scroll the link into view if needed (index is short; belt+braces)
    scroll = 0
    while y + h > scroll + VIEW_H:
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down,
                     "key": {"type": "qcode", "data": "pgdn"}}}])
        scroll += VIEW_H - 24
    if scroll == 0:
        check_px(img, "index: link underline color",
                 CONTENT_X + x + w // 2, PAGE_Y + y + h - 1, LINK_RGB)

    # --- 2. click the link -> page2 over our own TCP -------------------
    mark = len(d.serial())
    d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2 - scroll)
    check("click hit the link", d.wait_serial("BROWSER: clicked link -> /page2.htm", 10, mark))
    check("page2 rendered", d.wait_serial("BROWSER: rendered /page2.htm", 30, mark))
    check("M16_OK emitted", d.wait_serial("M16_OK", 5))
    log = d.serial()
    img2 = d.dump("m16_page2")
    check_px(img2, "page2: stylesheet body bg", CONTENT_X + 470, PAGE_Y + 6, BODY_BG)

    # --- 3. scroll if page2 overflows the viewport ----------------------
    info2 = rendered(log, "/page2.htm", mark)
    check("page2 render stats parsed", bool(info2), str(info2))
    if info2 and info2[3] > VIEW_H:
        smark = len(d.serial())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down,
                     "key": {"type": "qcode", "data": "pgdn"}}}])
        check("page2 scrolls", d.wait_serial("BROWSER: scroll y=", 5, smark))
        # scroll back up so link coordinates are view-space again
        for _ in range(20):
            for down in (True, False):
                d.send([{"type": "key", "data": {"down": down,
                         "key": {"type": "qcode", "data": "pgup"}}}])
        d.dump("m16_page2_scrolled")

    # --- 4. click 'back home' -> index again ----------------------------
    back = [b for b in boxes(log, "link", mark) if b[0] == "/index.htm"]
    check("back-home link laid out", len(back) == 1, str(back))
    if back:
        _, x, y, w, h = back[0]
        scroll = scroll_into_view(d, y, h)
        mark = len(d.serial())
        d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2 - scroll)
        check("back-home navigates", d.wait_serial("BROWSER: rendered /index.htm", 30, mark))
        d.dump("m16_back_home")

    d.quit()
    finish()


if __name__ == "__main__":
    main()
