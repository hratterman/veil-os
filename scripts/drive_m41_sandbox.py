#!/usr/bin/env python3
"""M41 step 15 — memory protection between processes.

Two negative tests:
  1) A malicious WASM app (EVIL.WSM) reads an address far past its own linear
     memory (reaching for kernel RAM). The interpreter sandbox blocks it, the app
     is killed cleanly, and the OS + every other app keep running.
  2) An EL0 binary (EVIL.BIN) reads kernel memory directly. The MMU faults, the
     kernel kills the process (USER FAULT -> exit_current), and never panics.
Throughout, there must be no `KERNEL PANIC` and other apps must still launch.
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
    for _ in range(60):  # reset the selection to the top
        key(d, "up")
    for _ in range(idx):
        key(d, "down")
    key(d, "ret")
    return True


def type_str(d, s):
    for ch in s:
        q = {" ": "spc"}.get(ch, ch.lower())
        key(d, q)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())

    # --- 1) malicious WASM app ------------------------------------------------
    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("file manager launched", d.wait_serial("WM: launch 'files'", 5, m))
    check("files listed", d.wait_serial("FILES[0]:", 4, m))

    m = len(d.serial())
    check("EVIL.WSM present", open_file(d, "EVIL.WSM"))
    check("sandbox blocked the OOB read",
          d.wait_serial("WASM_SANDBOX: blocked out-of-bounds load", 6, m))
    check("the app was killed cleanly",
          d.wait_serial("WASM_KILLED: EVIL.WSM trapped cleanly", 6, m))

    # The OS must still be alive: launch another app and confirm it runs.
    d.click(*taskbar_xy(d, "files"))  # re-raise the file manager
    m = len(d.serial())
    check("HELLOAPP.WSM present", open_file(d, "HELLOAPP.WSM"))
    check("a normal app still runs after the kill",
          d.wait_serial("WASMAPP_OK: ran HELLOAPP.WSM", 6, m))

    # --- 2) malicious EL0 process ---------------------------------------------
    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))
    m = len(d.serial())
    type_str(d, "evil")
    key(d, "ret")
    check("EL0 process faulted reading kernel memory",
          d.wait_serial("USER FAULT", 6, m))
    check("no privilege breach (kernel memory unreadable)",
          "PRIVILEGE BREACH" not in d.serial())

    # The OS never panicked through any of this.
    check("OS did not panic", "KERNEL PANIC" not in d.serial())
    # And the shell still works after killing the EL0 process.
    m = len(d.serial())
    type_str(d, "echo alive")
    key(d, "ret")
    check("shell still responsive after the kill",
          d.wait_serial("SHELL_OUT: alive", 5, m))

    d.move(1000, 700)
    d.dump("m41_sandbox")
    d.quit()
    finish()


main()
