#!/usr/bin/env python3
"""M35 real shell: ls / cat / cp / echo>file / cat / pipe against FAT16.
Shell launcher is index 4 with no NIC: x = 70 + 4*78 + 36 = 418."""
import sys

from guilib import Driver, check, finish

SHELL_BTN = (418, 768 - 20)


def type_str(d, s):
    m = {"/": "slash", ".": "dot", " ": "spc", ">": "dot", "|": "backslash", "-": "minus"}
    # '>' needs shift+dot, '|' needs shift+backslash
    for ch in s:
        if ch == ">":
            keys = [("shift", True), ("dot", True), ("dot", False), ("shift", False)]
        elif ch == "|":
            keys = [("shift", True), ("backslash", True), ("backslash", False), ("shift", False)]
        else:
            q = m.get(ch, ch.lower())
            keys = [(q, True), (q, False)]
        for q, down in keys:
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": q}}}])


def run(d, line):
    mark = len(d.serial())
    type_str(d, line)
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "ret"}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "ret"}}}])
    d.wait_serial("SHELL: $", 3, mark)
    return mark


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*SHELL_BTN)
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))

    m = run(d, "ls")
    check("ls lists files (README.TXT present)",
          d.wait_serial("SHELL_OUT: README.TXT", 4, m))

    m = run(d, "echo veil-m35 > test.txt")
    m = run(d, "cat test.txt")
    check("echo > file then cat reads it back",
          d.wait_serial("SHELL_OUT: veil-m35", 4, m))

    m = run(d, "cp test.txt copy.txt")
    m = run(d, "cat copy.txt")
    check("cp copies file contents", d.wait_serial("SHELL_OUT: veil-m35", 4, m))

    m = run(d, "ls")
    check("new files appear in ls", d.wait_serial("SHELL_OUT: TEST.TXT", 4, m))

    m = run(d, "cat copy.txt | grep veil")
    check("pipe: cat | grep filters", d.wait_serial("SHELL_OUT: veil-m35", 4, m))

    m = run(d, "rm copy.txt")
    m = run(d, "cat copy.txt")
    check("rm deletes the file", d.wait_serial("SHELL_OUT: cat: copy.txt: no such file", 4, m))

    d.move(1000, 700)
    d.dump("m35_shell")
    d.quit()
    finish()


main()
