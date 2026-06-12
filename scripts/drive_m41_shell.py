#!/usr/bin/env python3
"""M41 step 9 — real (bash-subset) shell.

Boot proves the interpreter (SHELL_OK: vars/arith/for/while/if/case/functions/
pipes/cmd-subst). Here the GUI shell runs the seeded TEST.SH — a non-trivial
script that builds three files, iterates them in a for-loop, pipes the merged
output through `sort -r`, writes it to out.txt, and reports counts — then a few
interactive one-liners (arithmetic, pipe). Output is checked via SHELL_OUT.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

# Shift-symbol table for the qcode keyboard (digit-row symbols).
SHIFT = {"$": "4", "(": "9", ")": "0", "*": "8", "!": "1", "&": "7", "%": "5"}
PLAIN = {"/": "slash", ".": "dot", " ": "spc", "-": "minus", "=": "equal", "'": "apostrophe"}


def key(d, q, down):
    d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": q}}}])


def type_str(d, s):
    for ch in s:
        if ch == ">":
            seq = [("shift", True), ("dot", True), ("dot", False), ("shift", False)]
        elif ch == "|":
            seq = [("shift", True), ("backslash", True), ("backslash", False), ("shift", False)]
        elif ch in SHIFT:
            q = SHIFT[ch]
            seq = [("shift", True), (q, True), (q, False), ("shift", False)]
        else:
            q = PLAIN.get(ch, ch.lower())
            seq = [(q, True), (q, False)]
        for q, down in seq:
            key(d, q, down)


def run(d, line):
    mark = len(d.serial())
    type_str(d, line)
    key(d, "ret", True)
    key(d, "ret", False)
    d.wait_serial("SHELL: $", 4, mark)
    return mark


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("SHELL_OK boot self-test", "SHELL_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))

    # The non-trivial script.
    m = run(d, "sh test.sh")
    check("script: file count", d.wait_serial("SHELL_OUT: files=3", 5, m))
    check("script: sort -r (cherry first)", d.wait_serial("SHELL_OUT: cherry", 5, m))
    check("script: sorted output has apple", d.wait_serial("SHELL_OUT: apple", 5, m))
    check("script: wc -l counted lines", d.wait_serial("SHELL_OUT: lines=3", 5, m))
    check("script: conditional passed", d.wait_serial("SHELL_OUT: RESULT_OK", 5, m))

    # Read the written output back.
    m = run(d, "cat out.txt")
    check("written output persisted (banana)", d.wait_serial("SHELL_OUT: banana", 4, m))

    # Interactive arithmetic + pipe.
    m = run(d, "echo $((6 * 7))")
    check("arithmetic expansion", d.wait_serial("SHELL_OUT: 42", 4, m))

    m = run(d, "ls | grep TXT | wc -l")
    check("pipe chain runs", d.wait_serial("SHELL: $", 4, m))

    d.move(1000, 700)
    d.dump("m41_shell")
    d.quit()
    finish()


main()
