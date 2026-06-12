#!/usr/bin/env python3
"""M41 step 21 — self-hosting: compile + run C inside Veil.

Boot self-test: the on-OS C compiler builds + runs a C program (CC_OK). Then the
GUI shell compiles the seeded HELLO.C with `cc hello.c` — the compiler emits a
WASM module, writes it to disk, and runs it, printing "Hello, Veil!" and 55. No
host machine: editor + compiler (cc) + shell + WASM runtime form the dev env.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

PLAIN = {" ": "spc", ".": "dot", "/": "slash", "-": "minus"}


def key(d, q):
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": q}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": q}}}])


def type_str(d, s):
    for ch in s:
        key(d, PLAIN.get(ch, ch.lower()))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("CC_OK boot self-test (compile + run C in Veil)", "CC_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))

    # Compile + run the seeded C program with the on-OS compiler.
    m = len(d.serial())
    type_str(d, "cc hello.c")
    key(d, "ret")
    check("cc compiled hello.c to WASM", d.wait_serial("CC: hello.c -> HELLO.WSM", 6, m))
    check("compiled program printed 'Hello, Veil!'", d.wait_serial("SHELL_OUT: Hello, Veil!", 6, m))
    check("compiled program computed 55", d.wait_serial("SHELL_OUT: 55", 6, m))

    # The compiler wrote a runnable WASM module to disk.
    m = len(d.serial())
    type_str(d, "ls")
    key(d, "ret")
    check("HELLO.WSM written to disk", d.wait_serial("SHELL_OUT: HELLO.WSM", 5, m))

    d.move(1000, 700)
    d.dump("m41_cc")
    d.quit()
    finish()


main()
