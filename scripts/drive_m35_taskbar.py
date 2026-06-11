#!/usr/bin/env python3
"""M35.5 taskbar overflow fix: with a NIC there are 15 launchers. Confirm every
pill fits on screen (with room for the clock) and that the LAST pill — which
used to be pushed off the 1024px edge — is clickable (hit-test matches render)."""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

SCREEN_W = 1024


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())

    pills = []
    for line in d.serial().splitlines():
        m = re.match(r"TASKBAR_PILL: (\S+) (\d+) (\d+)\s*$", line)
        if m:
            pills.append((m.group(1), int(m.group(2)), int(m.group(3))))
    check("taskbar pills logged", len(pills) >= 12, f"{len(pills)} pills")
    check("15 launchers present (NIC -> includes chat)", len(pills) == 15, f"{len(pills)} pills")

    # Every pill must fit on screen with room reserved for the clock.
    rightmost = max(x + w for _, x, w in pills)
    check("all pills fit on screen (no overflow)", rightmost <= SCREEN_W - 50,
          f"rightmost pill ends at {rightmost}px of {SCREEN_W}")

    # No overlaps (each pill starts at/after the previous one's end).
    ordered = sorted(pills, key=lambda p: p[1])
    ok = all(ordered[i][1] >= ordered[i - 1][1] + ordered[i - 1][2] for i in range(1, len(ordered)))
    check("pills do not overlap", ok)

    # The clock must have visible room on the far right.
    check("clock has room on the right", rightmost < SCREEN_W - 45)

    # Click the LAST pill (the one that used to overflow) — hit-test must match.
    last_app = ordered[-1][0]
    m = len(d.serial())
    d.click(*taskbar_xy(d, last_app))
    check(f"last pill '{last_app}' is clickable (hit-test matches render)",
          d.wait_serial(f"WM: taskbar -> '{last_app}'", 5, m))

    d.move(1000, 700)
    d.dump("m355_taskbar")
    d.quit()
    finish()


main()
