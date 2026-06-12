#!/usr/bin/env python3
"""M41 step 12 — Veil app SDK: run the compiled example.

HELLOAPP.WSM is the SDK's hello-rust example, compiled with the real toolchain
(`cargo build --target wasm32-unknown-unknown`) against `veil-sdk`. Opening it
runs `init`+`render` (a graphical app drawing text + a button via the veil_*
ABI); clicking the button dispatches `on_click`, which logs an incrementing
counter. Proves the SDK + graphical ABI end to end.
"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy


def key(d, q):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": q}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": q}}}])


def open_file(d, fname):
    idx = None
    for line in d.serial().splitlines():
        m = re.search(rf"FILES\[(\d+)\]: {re.escape(fname)}", line)
        if m:
            idx = int(m.group(1))
    if idx is None:
        return False
    for _ in range(idx):
        key(d, "down")
    key(d, "ret")
    return True


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("file manager launched", d.wait_serial("WM: launch 'files'", 5, m))
    check("files listed", d.wait_serial("FILES[0]:", 4, m))

    m = len(d.serial())
    check("HELLOAPP.WSM present", open_file(d, "HELLOAPP.WSM"))
    check("graphical Veil app detected", d.wait_serial("graphical Veil app", 6, m))
    check("app ran (init+render)", d.wait_serial("WASMAPP_OK: ran HELLOAPP.WSM", 6, m))

    img = d.dump("m41_sdk_open")

    # The window is at (180,80) 460x240; content origin = +2 border +22 title.
    # The app surface is drawn at the canvas top-left; the button is at
    # surface (20,104)-(170,148). Click its center.
    win_x, win_y = 180, 80
    cx = win_x + 2 + (20 + 75)
    cy = win_y + 2 + 22 + (104 + 22)
    m = len(d.serial())
    d.click(cx, cy)
    check("first click -> counter 1", d.wait_serial("WASM_APP: clicks=1", 5, m))
    m = len(d.serial())
    d.click(cx, cy)
    check("second click -> counter 2", d.wait_serial("WASM_APP: clicks=2", 5, m))

    d.move(1000, 700)
    d.dump("m41_sdk_clicked")
    d.quit()
    finish()


main()
