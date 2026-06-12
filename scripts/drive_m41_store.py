#!/usr/bin/env python3
"""M41 step 13 — in-OS app store / loader.

The App Store lists installed .WSM apps and installs new ones from a URL. The URL
field defaults to a fetchable path; clicking Install downloads the .wasm over the
kernel HTTP stack, validates the magic, and saves it as APP1.WSM (a fresh app).
Pressing Enter then runs the newly-installed app from the store — proving:
paste URL -> install -> appears in the list -> run.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

# Store window: (200,70) 460x420; content origin = +2 border, +2 +22 title.
WIN_X, WIN_Y = 200, 70
CX, CY = WIN_X + 2, WIN_Y + 2 + 22


def key(d, q):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": q}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": q}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "store"))
    check("app store launched", d.wait_serial("WM: launch 'store'", 5, m))
    check("store listed installed apps", d.wait_serial("STORE: ", 4, m))

    # Click the Install button (top-right of the window). URL defaults to a
    # fetchable .wasm path, so no typing needed.
    m = len(d.serial())
    d.click(CX + 456 - 52, CY + 71)
    check("install fetched + saved the app", d.wait_serial("STORE_INSTALL_OK: APP1.WSM", 10, m))

    # The newly-installed app is selected; Enter runs it.
    m = len(d.serial())
    key(d, "ret")
    check("installed app runs from the store", d.wait_serial("WASMAPP_OK: ran APP1.WSM", 8, m))
    check("it's the graphical Hello-Veil app", d.wait_serial("graphical Veil app", 8, m))

    d.move(1000, 700)
    d.dump("m41_store")
    d.quit()
    finish()


main()
