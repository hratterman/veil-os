#!/usr/bin/env python3
"""M41 step 5 — browser text selection + copy/paste.

Loads the local homepage in the browser, drags to select a region of page text
(highlighted blue), Ctrl+C copies the selection to the OS clipboard, then opens
the editor and Ctrl+V pastes it in. Also verifies Ctrl+A (select all) → Ctrl+C
copies the whole page. With a NIC, the browser/editor are taskbar launchers."""
import sys

from guilib import Driver, check, finish, taskbar_xy

# Browser window: launched at (510,30) 480x620, BORDER=2 TITLE_H=22, CHROME=42.
# Page content begins at screen (512, 96). Pick a wide swath of body text.
WIN_X, WIN_Y = 510, 30
CONTENT_X = WIN_X + 2          # +BORDER
CONTENT_Y = WIN_Y + 2 + 22     # +BORDER +TITLE_H
PAGE_TOP = CONTENT_Y + 42      # +CHROME


def keyev(d, key, down):
    d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def combo(d, mod, key):
    keyev(d, mod, True)
    keyev(d, key, True)
    keyev(d, key, False)
    keyev(d, mod, False)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # --- drag-select a region of page text -------------------------------------
    m = len(d.serial())
    x0, y0 = CONTENT_X + 30, PAGE_TOP + 120
    x1, y1 = CONTENT_X + 360, PAGE_TOP + 300
    d.drag(x0, y0, x1, y1, steps=12)
    selected = d.wait_serial("BROWSER: selected", 4, m)
    check("drag selected page text", selected)

    # If the drag landed on a link (navigation) instead, fall back: Ctrl+A.
    if selected:
        m = len(d.serial())
        combo(d, "ctrl", "c")
        check("Ctrl+C copied the selection", d.wait_serial("BROWSER: Ctrl+C copied", 4, m))

    # --- Ctrl+A select all, then Ctrl+C ---------------------------------------
    m = len(d.serial())
    combo(d, "ctrl", "a")
    combo(d, "ctrl", "c")
    check("Ctrl+A + Ctrl+C copied the whole page", d.wait_serial("BROWSER: Ctrl+C copied", 4, m))
    check("clipboard received bytes", d.wait_serial("CLIPBOARD: copied", 4, m))

    d.dump("m41_select_browser")

    # --- paste into the editor -------------------------------------------------
    m = len(d.serial())
    d.click(*taskbar_xy(d, "edit"))
    check("editor launched", d.wait_serial("WM: launch 'edit'", 5, m))
    m = len(d.serial())
    combo(d, "ctrl", "v")
    check("Ctrl+V pasted into editor", d.wait_serial("CLIPBOARD: pasted", 4, m))

    d.move(1000, 700)
    d.dump("m41_select_editor")
    d.quit()
    finish()


main()
