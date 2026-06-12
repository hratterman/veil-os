#!/usr/bin/env python3
"""M41 step 18 — kernel hardening.

Boot self-tests (deterministic): a boot-stack canary (STACK_CANARY_OK), per-boot
heap ASLR (ASLR_OK with a randomized base), and a W^X test — mapping a page
writable-but-non-executable, writing code into it, and executing it faults
cleanly (WXN_OK, caught by the exception handler). The OS keeps running (WM_OK)
after the controlled fault.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    s = d.serial()
    check("STACK_CANARY_OK (boot-stack overflow guard)", "STACK_CANARY_OK" in s)
    check("ASLR_OK (kernel heap randomized per boot)", "ASLR_OK" in s)
    check("WXN_OK (executing writable memory faulted cleanly)", "WXN_OK" in s)
    check("the W^X fault was caught + recovered", "HARDEN: caught expected" in s)
    check("OS booted normally through the hardening (WM_OK)", "WM_OK" in s)
    check("no kernel panic", "KERNEL PANIC" not in s)

    # The OS is fully alive after the controlled fault: launch an app.
    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("OS still responsive (shell launches)", d.wait_serial("WM: launch 'shell'", 5, m))

    d.move(1000, 700)
    d.dump("m41_harden")
    d.quit()
    finish()


main()
