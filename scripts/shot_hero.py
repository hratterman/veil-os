"""Open a spread of apps via the taskbar and screendump a hero shot for
the README. Launches Editor, Clock, Paint and Shell (tiling the desktop),
draws a couple of paint strokes, then dumps shots/hero.png."""
import sys
import time

from guilib import Driver

# Taskbar buttons: x = 70 + idx*78 + 36, y at the bottom 40px strip.
def btn(i):
    return (70 + i * 78 + 36, 768 - 20)

EDITOR, CLOCK, BROWSER, PAINT, SHELL = (btn(i) for i in range(5))


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    for b in (EDITOR, CLOCK, SHELL, PAINT):
        d.click(*b)
        time.sleep(0.4)
    # A few brush strokes in the paint window (content starts ~482,354).
    d.drag(540, 420, 760, 520, steps=20)
    d.drag(560, 560, 900, 470, steps=20)
    time.sleep(0.5)
    d.move(1010, 700)   # park the cursor out of the way
    time.sleep(1.2)     # let the clock tick render
    d.dump("hero")
    print("hero shot written")


if __name__ == "__main__":
    main()
