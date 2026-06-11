#!/usr/bin/env python3
"""M35 app kill: open browser + lisp, kill the browser from the shell, and
confirm the lisp REPL keeps running (still evaluates). NIC taskbar indices:
browser 2 (262), shell 4 (418), lisp 10 (886)."""
import sys

from guilib import Driver, check, finish

LISP_BTN = (886, 748)
BROWSER_BTN = (262, 748)
SHELL_BTN = (418, 748)


def keyev(d, key, down):
    d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def type_str(d, s):
    for ch in s:
        q = {" ": "spc"}.get(ch, ch.lower())
        keyev(d, q, True)
        keyev(d, q, False)


def enter(d):
    keyev(d, "ret", True)
    keyev(d, "ret", False)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())

    m = len(d.serial())
    d.click(*LISP_BTN)
    check("lisp launched", d.wait_serial("WM: launch 'lisp'", 5, m))
    m = len(d.serial())
    d.click(*BROWSER_BTN)
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    m = len(d.serial())
    d.click(*SHELL_BTN)
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))

    # Kill the browser from the shell.
    m = len(d.serial())
    type_str(d, "kill browser")
    enter(d)
    check("browser killed from shell", d.wait_serial("WM: killed 'browser'", 4, m))

    # The lisp REPL must still be alive: focus it and evaluate.
    m = len(d.serial())
    d.click(300, 200)  # lisp window content
    type_str(d, "99")
    enter(d)
    check("lisp REPL still evaluates after kill",
          d.wait_serial("LISP_EVAL: 99 => 99", 5, m))

    d.move(1000, 700)
    d.dump("m35_kill")
    d.quit()
    finish()


main()
