#!/usr/bin/env python3
"""M35 WASM: open HELLO.WSM from the file manager -> it runs and prints output;
open COMPUTE.WSM -> a JIT-compiled compute kernel runs. No-NIC: files idx 7."""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

FILES_BTN = (652, 748)
CONTENT_X, CONTENT_Y = 122, 84
ROW_H = 14
COL_W = 164


def open_file(d, fname):
    idx = None
    for line in d.serial().splitlines():
        m = re.search(rf"FILES\[(\d+)\]: {re.escape(fname)}", line)
        if m:
            idx = int(m.group(1))
    if idx is None:
        return False
    rows = (378 - 24) // ROW_H
    col, row = idx // rows, idx % rows
    d.click(CONTENT_X + col * COL_W + 80, CONTENT_Y + row * ROW_H + 7)
    return True


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("file manager launched", d.wait_serial("WM: launch 'files'", 5, m))
    check("files listed", d.wait_serial("FILES[0]:", 4, m))

    # hello-world WASM: runs and prints via fd_write.
    m = len(d.serial())
    check("HELLO.WSM present", open_file(d, "HELLO.WSM"))
    check("wasm runtime opened", d.wait_serial("FILES: open HELLO.WSM in WASM runtime", 5, m))
    check("hello wasm printed via fd_write",
          d.wait_serial("WASMAPP: HELLO.WSM _start printed", 6, m))
    d.move(1000, 700)
    img = d.dump("m35_wasm")

    # compute WASM: JIT-compiled kernel runs. Re-raise the file manager first
    # (the hello window overlaps the list).
    d.click(*taskbar_xy(d, "files"))
    m = len(d.serial())
    check("COMPUTE.WSM present", open_file(d, "COMPUTE.WSM"))
    check("compute wasm JIT ran",
          d.wait_serial("WASMAPP: COMPUTE.WSM compute =", 6, m)
          and "jit=true" in d.serial()[m:])
    d.move(1000, 700)
    d.dump("m35_wasm_compute")

    d.quit()
    finish()


main()
