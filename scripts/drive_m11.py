#!/usr/bin/env python3
"""M11 proof: a shell in a window runs user binaries off the filesystem,
preemptively multitasked, while staying responsive.

Checks: `ls` lists files, `cat` prints one, `echo` echoes, a busy-looping
`spin` (which never yields) keeps running WHILE a later `echo` executes —
preemption — and `paint` opens a new window."""
import sys
import time
from guilib import Driver, check, check_px, finish

T_FOCUS = (48, 96, 192)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    for sentinel in ["SHELL_OK", "M11_OK", "M10_OK"]:
        check(f"serial sentinel {sentinel}", sentinel in d.serial())

    d.click(150, 600)  # focus the shell window
    mark = len(d.serial())

    print("--- ls: list files from the shell ----------------------------")
    d.type_text("ls\n")
    check("ls spawned from disk", d.wait_serial("SCHED: spawn", after=mark))
    for f in ["README.TXT 64", "HELLO.BIN", "LS.BIN", "CAT.BIN", "ECHO.BIN", "SPIN.BIN"]:
        check(f"ls output contains {f.split()[0]}", d.wait_serial(f, 10, after=mark))

    print("--- cat: print a file -----------------------------------------")
    mark = len(d.serial())
    d.type_text("cat README.TXT\n")
    check("cat printed the file",
          d.wait_serial("Hello from the Veil filesystem! This file was written by macOS.",
                        after=mark))

    print("--- echo --------------------------------------------------------")
    mark = len(d.serial())
    d.type_text("echo veil shell says hi\n")
    check("echo output", d.wait_serial("veil shell says hi", after=mark))

    print("--- preemption: spin busy-loops while echo still runs ----------")
    mark = len(d.serial())
    d.type_text("spin 20\n")
    check("spin started", d.wait_serial("beat 1/20", 30, after=mark))
    d.type_text("echo shell still responsive\n")
    check("echo ran during spin", d.wait_serial("shell still responsive", 30, after=mark))
    log = d.serial()[mark:]
    echo_pos = log.find("shell still responsive")
    spin_done = log.find(": done")
    check("spin still running when echo executed (preemption)",
          echo_pos > -1 and (spin_done == -1 or echo_pos < spin_done))
    check("spin eventually finished", d.wait_serial(": done", 120, after=mark))
    check("spin exit reaped", d.wait_serial("exited with code 0", after=mark))

    print("--- paint: launch as a window from the shell --------------------")
    mark = len(d.serial())
    d.type_text("paint\n")
    check("paint launched", d.wait_serial("SHELL: launched 'paint-1'", after=mark))
    img = d.dump("m11_desktop")
    # paint-1 spawns at (544, 60); its title bar spans y 62..84.
    check_px(img, "new paint window title bar on screen", 900, 70, T_FOCUS)

    # The shell window should be full of rendered text by now.
    lit = sum(1 for y in range(455, 730, 3) for x in range(45, 455, 3)
              if img.at(x, y) == (208, 216, 224))
    check("shell window shows rendered output", lit > 40, f"{lit} text pixels (sampled)")

    d.quit()
    finish()


if __name__ == "__main__":
    main()
