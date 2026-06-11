#!/usr/bin/env python3
"""Showcase the M35.5 redesigned desktop: open a browser (scrollbar), files, and
a game, then screenshot. NIC taskbar: browser=262, files=652, snake=886."""
import sys
from guilib import Driver, finish

def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial()); d.click(262, 752)  # browser
    d.wait_serial("BROWSER: rendered / -", 40)
    d.click(652, 752)  # files
    d.wait_serial("WM: launch 'files'", 5)
    d.click(886, 752)  # snake
    d.wait_serial("SNAKE: new game", 5)
    d.move(1000, 700)
    d.dump("m355_showcase")
    d.quit()
    print("ALL CHECKS PASSED")
    finish()
main()
