#!/usr/bin/env python3
"""M41 step 10 — standalone binaries (grep/sed/awk/cut/tr/curl).

Boot proves the tools through pipes (COREUTILS_OK). Here the GUI shell runs the
pass-condition pipelines: `ls | grep <regex> | wc -l`, a sed transform, and curl
piped to grep — `curl /index.htm | grep Veil` (deterministic, loopback) and the
acceptance `curl https://henryratterman.com | grep Henry`.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

# digit-row + punctuation shift symbols
SHIFT = {"$": "4", "(": "9", ")": "0", "*": "8", "!": "1", "&": "7", "%": "5",
         ":": "semicolon", '"': "apostrophe", "+": "equal", "?": "slash"}
PLAIN = {"/": "slash", ".": "dot", " ": "spc", "-": "minus", "=": "equal",
         "'": "apostrophe", ",": "comma", ";": "semicolon"}


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
        elif ch.isupper():
            q = ch.lower()
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
    d.wait_serial("SHELL: $", 5, mark)
    return mark


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("COREUTILS_OK boot self-test", "COREUTILS_OK" in d.serial())

    m = len(d.serial())
    d.click(*taskbar_xy(d, "shell"))
    check("shell launched", d.wait_serial("WM: launch 'shell'", 5, m))

    # ls | grep <regex> — anchored ".TXT$"
    m = run(d, "ls | grep .TXT$")
    check("ls | grep regex finds .TXT files", d.wait_serial("SHELL_OUT: README.TXT", 4, m))

    # the pass-condition pipe shape: ls | grep <regex> | wc -l
    m = run(d, "ls | grep .RS$ | wc -l")
    check("ls | grep | wc -l pipe chain ran", d.wait_serial("SHELL: $", 4, m))

    # sed substitution over a file
    m = run(d, "cat demo.rs | sed s/fn/FUNC/g")
    check("sed s///g substitutes", d.wait_serial("SHELL_OUT: FUNC main", 4, m))

    # curl loopback | grep (deterministic)
    m = run(d, "curl /index.htm | grep Veil")
    check("curl /index.htm fetched", d.wait_serial("CURL: /index.htm -> 200", 8, m))
    check("curl | grep finds 'Veil'", d.wait_serial("SHELL_OUT: <h1>Veil", 4, m))

    # acceptance: curl https://henryratterman.com | grep Henry
    m = run(d, "curl https://henryratterman.com | grep Henry")
    fetched = d.wait_serial("CURL: https://henryratterman.com -> 200", 25, m)
    check("curl https reached henryratterman.com", fetched)
    if fetched:
        check("curl https | grep finds 'Henry'", d.wait_serial("Henry Ratterman", 6, m))

    d.move(1000, 700)
    d.dump("m41_coreutils")
    d.quit()
    finish()


main()
