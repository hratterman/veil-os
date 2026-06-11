#!/usr/bin/env python3
"""M31 GIF player proof: launch the GIF app, confirm GIF_OK and that the
demo decodes (64x64, 12 frames), that it animates while playing, and that
Space freezes it."""
import sys
import time

from guilib import Driver, check, finish, taskbar_xy

# GIF is launcher idx 8 without a NIC (chat filtered):
# edit,clock,browser,paint,shell,viewer,audio,files,gif -> x=70+8*78+36=730.
GIF_BTN = (70 + 8 * 78 + 36, 768 - 20)
CX, CY, CW, CH = 202, 114, 280, 240  # window (200,90) content box


def key(d, qc):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qc}}}])


def region(img):
    rows = []
    for y in range(CY, CY + CH):
        i = (y * img.w + CX) * 3
        rows.append(img.px[i:i + CW * 3])
    return b"".join(rows)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    mark = len(d.serial())
    d.click(*taskbar_xy(d, "gif"))
    check("gif launched", d.wait_serial("WM: launch 'gif'", 5, mark))
    check("GIF_OK on serial", d.wait_serial("GIF_OK", 5, mark))
    check("demo decoded 64x64 / 12 frames",
          d.wait_serial("GIF: DEMO.GIF 64x64, 12 frames", 5, mark))

    d.move(1000, 700)
    a = region(d.dump("m31_gif_play_a"))
    time.sleep(0.7)
    b = region(d.dump("m31_gif_play_b"))
    check("animates while playing (frames differ)", a != b)

    key(d, "spc")  # pause
    time.sleep(0.2)
    c = region(d.dump("m31_gif_pause_c"))
    time.sleep(0.7)
    e = region(d.dump("m31_gif_pause_d"))
    check("paused = frozen (frames identical)", c == e)

    key(d, "spc")  # resume
    time.sleep(0.7)
    f = region(d.dump("m31_gif_resume"))
    check("resumes animating", f != c)
    d.quit()
    finish()


if __name__ == "__main__":
    main()
