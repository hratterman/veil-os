#!/usr/bin/env python3
"""M35 clipboard + Alt+Tab. Ctrl+C copies the browser's visible text; opening
the shell and Ctrl+V pastes it; Alt+Tab cycles windows. With a NIC: browser is
taskbar idx 2 (x=262), shell idx 4 (x=418)."""
import sys

from guilib import Driver, check, finish

BROWSER_BTN = (262, 748)
SHELL_BTN = (418, 748)


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
    d.click(*BROWSER_BTN)
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # Ctrl+C copies the page text.
    m = len(d.serial())
    combo(d, "ctrl", "c")
    check("Ctrl+C copied browser text", d.wait_serial("BROWSER: Ctrl+C copied", 4, m))

    # Open the shell and Ctrl+V to paste the copied text.
    m = len(d.serial())
    d.click(*SHELL_BTN)
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))
    m = len(d.serial())
    combo(d, "ctrl", "v")
    check("Ctrl+V pasted into shell", d.wait_serial("CLIPBOARD: pasted", 4, m))

    # Alt+Tab cycles between the two open windows.
    m = len(d.serial())
    combo(d, "alt", "tab")
    check("Alt+Tab switches windows", d.wait_serial("WM: Alt+Tab ->", 4, m))

    d.move(1000, 700)
    d.dump("m35_clip")
    d.quit()
    finish()


main()
