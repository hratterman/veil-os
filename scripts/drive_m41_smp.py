#!/usr/bin/env python3
"""M41 step 19 — SMP / multiple CPU cores.

Boot brings up the secondary cores via PSCI and runs a parallel CPU-bound
workload. We confirm: 4 cores come online (NPROC=4), the workload is faster on
all cores than on one (SMP_OK), and `nproc` in the shell reports 4.
"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

PLAIN = {" ": "spc", "/": "slash", ".": "dot"}


def key(d, q):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": q}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": q}}}])


def type_str(d, s):
    for ch in s:
        key(d, PLAIN.get(ch, ch.lower()))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    s = d.serial()
    check("secondary cores came online", "SMP: 4 core(s) online" in s)
    m = re.search(r"NPROC: (\d+)", s)
    check("NPROC reports 4 cores", bool(m) and int(m.group(1)) == 4)
    check("parallel workload is faster than 1 core (SMP_OK)", "SMP_OK" in s)
    check("no kernel panic from SMP bring-up", "KERNEL PANIC" not in s)

    # `nproc` in the shell reports 4.
    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))
    m = len(d.serial())
    type_str(d, "nproc")
    key(d, "ret")
    check("shell nproc reports 4", d.wait_serial("SHELL_OUT: 4", 5, m))

    d.move(1000, 700)
    d.dump("m41_smp")
    d.quit()
    finish()


main()
