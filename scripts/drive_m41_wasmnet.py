#!/usr/bin/env python3
"""M41 step 11 — network access for WASM apps.

NETGET.WSM imports the Veil host's `veil_http_get` and WASI `fd_write`. Opening
it from the file manager runs `_start`, which fetches http://example.com/ over
the kernel's HTTP/TCP stack and prints the response via fd_write. We confirm the
host network call fired (WASM_NET) and the external server's content reached the
app (WASM_OUT contains "Example Domain").
"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

def key(d, q):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": q}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": q}}}])


def open_file(d, fname):
    """Select `fname` by index via Down-arrow navigation, then Enter."""
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
    check("NETGET.WSM present", open_file(d, "NETGET.WSM"))
    check("wasm runtime opened", d.wait_serial("FILES: open NETGET.WSM in WASM runtime", 6, m))

    # The Veil host network call fired (external HTTP over the kernel stack).
    check("veil_http_get reached example.com",
          d.wait_serial("WASM_NET: veil_http_get http://example.com/ -> 200", 25, m))
    # The external server's response reached the WASM app and printed.
    check("WASM app displayed the response (Example Domain)",
          d.wait_serial("WASM_OUT: ", 6, m) and "Example Domain" in d.serial()[m:])

    d.move(1000, 700)
    d.dump("m41_wasmnet")
    d.quit()
    finish()


main()
