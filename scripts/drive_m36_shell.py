#!/usr/bin/env python3
"""M36 shell POSIX upgrade: find, wc, date, df, grep-on-file, && / || chains."""
import sys

from guilib import Driver, check, finish, taskbar_xy


def type_str(d, s):
    m = {"/": "slash", ".": "dot", " ": "spc", "-": "minus"}
    shifted = {">": "dot", "*": "8", "&": "7", "|": "backslash"}
    for ch in s:
        if ch in shifted:
            keys = [("shift", True), (shifted[ch], True), (shifted[ch], False), ("shift", False)]
        else:
            q = m.get(ch, ch.lower())
            keys = [(q, True), (q, False)]
        for q, down in keys:
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": q}}}])


def run(d, line):
    mark = len(d.serial())
    type_str(d, line)
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": "ret"}}}])
    return mark


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))

    m = run(d, "find / -name *.TXT")
    check("find lists .TXT files", d.wait_serial("SHELL_OUT: /README.TXT", 4, m))

    m = run(d, "wc -l README.TXT")
    check("wc -l counts lines", d.wait_serial("SHELL_OUT:", 4, m))

    m = run(d, "df")
    check("df shows FAT16", d.wait_serial("SHELL_OUT: FAT16", 4, m))

    m = run(d, "date")
    check("date prints", d.wait_serial("SHELL_OUT:", 4, m))

    run(d, "echo helloworld > gt.txt")
    m = run(d, "grep hello gt.txt")
    check("grep on a file", d.wait_serial("SHELL_OUT: helloworld", 4, m))

    m = run(d, "cat README.TXT && echo chainok")
    check("&& runs after success", d.wait_serial("SHELL_OUT: chainok", 4, m))

    m = run(d, "cat NOPE.TXT || echo orworks")
    check("|| runs after failure", d.wait_serial("SHELL_OUT: orworks", 4, m))

    d.move(900, 650)
    d.dump("m36_shell")
    d.quit()
    finish()


main()
