#!/usr/bin/env python3
"""M35 browser text input: editable address bar + on-page <input> fields.

  - Click the address bar, type a path, press Enter -> the browser navigates.
  - Open a form page, click the text field, type -> the value fills visibly.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

# Browser window at (510,30), BORDER 2, TITLE_H 22 -> content origin (512,54),
# then a 20px address bar; the page starts at y=74.
CONTENT_X, CONTENT_Y = 512, 54
PAGE_Y = CONTENT_Y + 42


def boxes(s, kind):
    import re
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(
                rf"BROWSER: {kind} '([^']+)'(?: name='[^']*')? at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]


def click_link(d, href_sub):
    lk = [b for b in boxes(d.serial(), "link") if href_sub in b[0]]
    if not lk:
        return False
    _, x, y, w, h = lk[-1]
    d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    return True


def type_str(d, s):
    for ch in s:
        qcode = {"/": "slash", ".": "dot", ":": "semicolon", "-": "minus"}.get(ch, ch.lower())
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

    # 1) Editable address bar: click it, type a path, Enter -> navigate.
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)  # the address bar
    type_str(d, "/news.htm")
    press(d, "ret")
    check("address bar navigates to typed path",
          d.wait_serial("BROWSER: rendered /news.htm", 20, m))

    # 2) On-page input field: open the form page, click the field, type.
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)
    type_str(d, "/formtest.htm")
    press(d, "ret")
    check("form page rendered", d.wait_serial("BROWSER: rendered /formtest.htm", 20, m))
    check("form fields detected", d.wait_serial("form field", 5, m))

    # Click the actual input box (logged with its doc position) and type.
    flds = boxes(d.serial(), "field")
    check("input field box logged", bool(flds))
    fx, fy, fw, fh = flds[0][1], flds[0][2], flds[0][3], flds[0][4]
    sx, sy = CONTENT_X + fx + fw // 2, PAGE_Y + fy + fh // 2
    d.click(sx, sy)
    type_str(d, "hello")
    d.move(1000, 700)
    img = d.dump("m35_input")
    # The typed light text should now appear as light pixels inside the box.
    found = any(img.at(x, y)[0] > 180 and img.at(x, y)[1] > 180 and img.at(x, y)[2] > 180
                for y in range(sy - 6, sy + 7)
                for x in range(CONTENT_X + fx + 2, CONTENT_X + fx + fw - 2))
    check("typed text visible in input field", found, f"field box @doc({fx},{fy})")

    d.quit()
    finish()


main()
