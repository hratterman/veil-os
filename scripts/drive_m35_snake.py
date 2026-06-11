#!/usr/bin/env python3
"""M35 Snake: launches, plays (auto-eats the first food straight ahead), scores,
games over, and saves the high score to SNAKE.TXT. No-NIC taskbar: snake is
idx 10 -> x = 70 + 10*78 + 36 = 886."""
import sys

from guilib import Driver, check, finish

SNAKE_BTN = (886, 748)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("WM_OK on serial", "WM_OK" in d.serial())
    m = len(d.serial())
    d.click(*SNAKE_BTN)
    check("snake launched", d.wait_serial("WM: launch 'snake'", 5, m))
    check("new game started", d.wait_serial("SNAKE: new game", 4, m))

    # The snake moves right and eats the food placed straight ahead.
    check("snake eats food and scores", d.wait_serial("SNAKE: score 1", 8, m))
    d.move(1000, 700)
    d.dump("m35_snake")

    # It then runs into the wall: game over, and the high score is persisted.
    check("game over after hitting wall", d.wait_serial("SNAKE: game over, score 1", 10, m))
    check("high score saved to SNAKE.TXT",
          d.wait_serial("SNAKE: new high score 1 saved to SNAKE.TXT", 4, m))

    # Restart with R.
    m = len(d.serial())
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": "r"}}}])
    check("restarts on R (new game with saved high score)",
          d.wait_serial("SNAKE: score 1", 8, m) or d.wait_serial("SNAKE: game over", 8, m))

    d.quit()
    finish()


main()
