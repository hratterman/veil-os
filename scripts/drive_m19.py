"""M19 proof: the clock window ticks on all four faces.

Per face: capture the clock content region at t, t+1s, t+2s and require
all three captures pairwise distinct (the second hand / digits moved).
The chrono and stopwatch faces only tick once STA is clicked — also
verified frozen (pixel-identical captures) while stopped, and that RST
is acknowledged. Face cycling is a click on the face; serial logs every
transition.

Clock geometry (desktop.rs): window at (700, 36), content 260x260, at the
BOTTOM of the z-order — first click raises it by the title bar. Buttons
(chrono/stopwatch): strip y = ch-30..ch-4, three buttons of (cw-16)/3 px.
"""
import sys
import time

from guilib import Driver, check, finish

# UX overhaul: nothing opens at boot — launch the clock from the taskbar.
# Clock is launcher index 1; buttons start at x=70, stride 78, width 72;
# the 40px taskbar sits at the bottom of the 768px screen.
CLOCK_BTN = (70 + 1 * 78 + 36, 768 - 20)

WIN_X, WIN_Y, CW, CH = 700, 36, 260, 260
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22
TITLE = (WIN_X + 130, WIN_Y + 12)
FACE = (CONTENT_X + 128, CONTENT_Y + 90)   # clear of the button strip
BW = (CW - 16) // 3
BTN_Y = CONTENT_Y + CH - 30 + 13
STA = (CONTENT_X + 4 + BW // 2, BTN_Y)
STP = (CONTENT_X + 8 + BW + BW // 2, BTN_Y)
RST = (CONTENT_X + 12 + 2 * BW + BW // 2, BTN_Y)
PARK = (1000, 700)


def region(img):
    rows = []
    for y in range(CONTENT_Y, CONTENT_Y + CH):
        i = (y * img.w + CONTENT_X) * 3
        rows.append(img.px[i:i + CW * 3])
    return b"".join(rows)


def capture(d, name):
    d.move(*PARK)
    return region(d.dump(name))


def ticking(d, face, n=3):
    """n captures ~2s apart must be pairwise distinct. The spacing leaves
    headroom over the digital face's 1 s resolution: guest ticks lag host
    wall time under TCG load, and captures 1 s apart can alias into the
    same displayed second."""
    snaps = []
    for i in range(n):
        if i:
            time.sleep(1.5)
        snaps.append(capture(d, f"m19_{face}_{i}"))
    for a in range(n):
        for b in range(a + 1, n):
            check(f"{face}: t={a}s vs t={b}s differ", snaps[a] != snaps[b])


def frozen(d, face):
    a = capture(d, f"m19_{face}_frozen_a")
    time.sleep(0.8)
    b = capture(d, f"m19_{face}_frozen_b")
    check(f"{face}: frozen while stopped", a == b)


def cycle(d, expect):
    mark = len(d.serial())
    d.click(*FACE)
    check(f"face cycled to {expect}", d.wait_serial(f"CLOCK: face -> {expect}", 5, mark))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("CLOCK_OK on serial", "CLOCK_OK" in d.serial())

    mark = len(d.serial())
    d.click(*CLOCK_BTN)  # launch the clock window from the taskbar
    check("clock launched", d.wait_serial("WM: launch 'clock'", 5, mark))

    d.click(*TITLE)  # raise the clock above everything

    ticking(d, "wall")
    cycle(d, "digital")
    ticking(d, "digital")

    cycle(d, "chrono")
    frozen(d, "chrono")           # engine not started yet
    mark = len(d.serial())
    d.click(*STA)
    check("chrono started", d.wait_serial("CLOCK: start", 5, mark))
    ticking(d, "chrono")
    mark = len(d.serial())
    d.click(*STP)
    check("chrono stopped", d.wait_serial("CLOCK: stop", 5, mark))

    cycle(d, "stopwatch")
    frozen(d, "stopwatch")        # shared accumulator, stopped
    mark = len(d.serial())
    d.click(*STA)
    check("stopwatch started", d.wait_serial("CLOCK: start", 5, mark))
    ticking(d, "stopwatch")
    mark = len(d.serial())
    d.click(*STP)
    check("stopwatch stopped", d.wait_serial("CLOCK: stop", 5, mark))
    d.click(*RST)
    check("stopwatch reset", d.wait_serial("CLOCK: reset", 5, mark))

    cycle(d, "wall")              # full cycle wraps around
    d.quit()
    finish()


if __name__ == "__main__":
    main()
