#!/usr/bin/env python3
"""M41 step 16 — capability-based security.

NETTRY.WSM is a non-system app that tries an HTTP GET each frame. On first launch
the OS shows a permission dialog. Without the network grant the kernel host
refuses the call (PERM_DENIED) and the probe reports net=denied — the app cannot
bypass it. Clicking Allow grants network+filesystem and the probe reports net=ok.
Settings -> Apps -> Revoke takes the grant back, and the probe is denied again.
"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

NET_X, NET_Y = 180, 80          # NETTRY window
NET_CX, NET_CY = NET_X + 2, NET_Y + 2 + 22
SET_X, SET_Y = 240, 110         # Settings window
SET_CX, SET_CY = SET_X + 2, SET_Y + 2 + 22


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
    for _ in range(60):
        key(d, "up")
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

    # 1) Launch the untrusted network app.
    m = len(d.serial())
    check("NETTRY.WSM present", open_file(d, "NETTRY.WSM"))
    check("permission dialog shown", d.wait_serial("PERMS: NETTRY.WSM requests", 6, m))
    check("kernel refused the network call (no grant)",
          d.wait_serial("PERM_DENIED:", 6, m))
    check("the app saw the denial (net=denied)", d.wait_serial("WASM_APP: net=denied", 6, m))

    # 2) Click Allow in the dialog (Allow button center).
    m = len(d.serial())
    d.click(NET_CX + 135, NET_CY + 127)
    check("network permission granted", d.wait_serial("PERMS: granted network", 6, m))
    check("now the network call succeeds (net=ok)", d.wait_serial("WASM_APP: net=ok", 6, m))

    # 3) Revoke in Settings -> Apps.
    m = len(d.serial())
    d.click(*taskbar_xy(d, "settings"))
    check("settings launched", d.wait_serial("WM: launch 'settings'", 5, m))
    d.click(SET_CX + 60, SET_CY + 120)   # sidebar: "Apps" page (index 3)
    m = len(d.serial())
    d.click(SET_CX + 410, SET_CY + 85)   # Revoke button on NETTRY's row
    check("permission revoked in Settings", d.wait_serial("PERMS: revoked", 6, m))

    # 4) Re-run the probe (click its window content, in the strip Settings does
    #    not cover) — raises NETTRY + dispatches on_click -> re-run, denied again.
    m = len(d.serial())
    d.click(NET_CX + 24, NET_CY + 96)
    check("after revoke the network call is denied again",
          d.wait_serial("WASM_APP: net=denied", 6, m))

    d.move(1000, 700)
    d.dump("m41_perms")
    d.quit()
    finish()


main()
