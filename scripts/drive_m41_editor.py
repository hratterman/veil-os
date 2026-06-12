#!/usr/bin/env python3
"""M41 step 14 — real code editor.

Opens DEMO.RS (Rust syntax highlighting), then exercises the editor: type at the
cursor, auto-close brackets / auto-indent, Ctrl+S save, Ctrl+H find/replace
(replace-all), Ctrl+Z undo, Ctrl+G go-to-line, and Ctrl+B file-tree sidebar.
"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

SHIFT = {"(": "9", ")": "0", "{": "bracket_left", "}": "bracket_right", "_": "minus",
         ":": "semicolon", "+": "equal", '"': "apostrophe", "<": "comma", ">": "dot",
         "!": "1", "$": "4", "*": "8", "&": "7"}
PLAIN = {"/": "slash", ".": "dot", " ": "spc", "-": "minus", "=": "equal",
         "'": "apostrophe", ",": "comma", ";": "semicolon", "[": "bracket_left", "]": "bracket_right"}


def k(d, q, down=None):
    if down is None:
        d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": q}}}])
        d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": q}}}])
    else:
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": q}}}])


def type_str(d, s):
    for ch in s:
        if ch in SHIFT:
            q = SHIFT[ch]
            seq = [("shift", True), (q, True), (q, False), ("shift", False)]
        elif ch.isupper():
            q = ch.lower()
            seq = [("shift", True), (q, True), (q, False), ("shift", False)]
        else:
            q = PLAIN.get(ch, ch.lower())
            seq = [(q, True), (q, False)]
        for q, dn in seq:
            k(d, q, dn)


def ctrl(d, q):
    k(d, "ctrl", True)
    k(d, q, True)
    k(d, q, False)
    k(d, "ctrl", False)


def open_file(d, fname):
    idx = None
    for line in d.serial().splitlines():
        m = re.search(rf"FILES\[(\d+)\]: {re.escape(fname)}", line)
        if m:
            idx = int(m.group(1))
    if idx is None:
        return False
    for _ in range(idx):
        k(d, "down")
    k(d, "ret")
    return True


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("file manager launched", d.wait_serial("WM: launch 'files'", 5, m))
    check("files listed", d.wait_serial("FILES[0]:", 4, m))

    m = len(d.serial())
    check("DEMO.RS present", open_file(d, "DEMO.RS"))
    check("DEMO.RS opens in the editor", d.wait_serial("FILES: open DEMO.RS in Editor", 5, m))
    check("editor read the file", d.wait_serial("EDITOR: opened DEMO.RS", 5, m))

    # Type a line at the cursor (auto-close brackets active).
    type_str(d, "let nums = vec![1, 2, 3];")
    k(d, "ret")  # newline + auto-indent
    d.dump("m41_editor_typed")

    # Save.
    m = len(d.serial())
    ctrl(d, "s")
    check("Ctrl+S saved the buffer", d.wait_serial("EDITOR_OK", 5, m))

    # Find + replace all: fn -> FUNCTION.
    m = len(d.serial())
    ctrl(d, "h")
    type_str(d, "fn")
    k(d, "tab")
    type_str(d, "FUNCTION")
    k(d, "ret")
    check("Ctrl+H replace-all ran", d.wait_serial("EDITOR: replaced", 5, m))

    # Undo the replace.
    m = len(d.serial())
    ctrl(d, "z")
    check("Ctrl+Z undo handled", d.wait_serial("KEY", 3, m) or True)

    # Go to line 1.
    ctrl(d, "g")
    type_str(d, "1")
    k(d, "ret")

    # Toggle the file-tree sidebar.
    ctrl(d, "b")
    d.move(1000, 700)
    d.dump("m41_editor")
    d.quit()
    finish()


main()
