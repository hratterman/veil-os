#!/usr/bin/env python3
"""M35 test 3: load en.wikipedia.org/wiki/QEMU, check article text + images."""
import sys
from guilib import Driver, check, finish

def keyev(d, key, down):
    d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])

def type_str(d, s):
    for ch in s:
        if ch == ":":
            ks = [("shift", True), ("semicolon", True), ("semicolon", False), ("shift", False)]
        elif ch == "/": ks = [("slash", True), ("slash", False)]
        elif ch == ".": ks = [("dot", True), ("dot", False)]
        elif ch == "_": ks = [("shift", True), ("minus", True), ("minus", False), ("shift", False)]
        else: ks = [(ch.lower(), True), (ch.lower(), False)]
        for q, dn in ks: keyev(d, q, dn)

def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial()); d.click(262, 748)
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    m = len(d.serial())
    d.click(650, 62)
    type_str(d, "https://en.wikipedia.org/wiki/QEMU")
    keyev(d, "ret", True); keyev(d, "ret", False)
    check("redirect followed (qemu -> Qemu)",
          d.wait_serial("BROWSER: following redirect", 60, m) or
          d.wait_serial("en.wikipedia.org/wiki/Qemu", 60, m))
    check("article text rendered",
          d.wait_serial("BROWSER: rendered https://en.wikipedia.org/wiki/Qemu", 90, m))
    import re
    items = 0
    mm = re.findall(r"rendered https://en.wikipedia.org/wiki/Qemu - (\d+) items", d.serial())
    if mm: items = max(int(x) for x in mm)
    check("article has substantial content", items > 500, f"{items} items")
    import time; time.sleep(1.5); d.move(1000, 700); d.dump("m35_wiki")
    # report decoded images + render stats
    for line in d.serial()[m:].splitlines():
        if any(k in line for k in ("decoded", "rendered https://en.wiki", "is not a PNG", "items,")):
            print("  " + line)
    d.quit(); finish()

main()
