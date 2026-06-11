#!/usr/bin/env python3
"""M23 proof: the image viewer shows a PNG decoded by the kernel's own
decoder and navigates with the arrow keys.

Launch the viewer from the taskbar; it opens on CHECK.PNG (first
alphabetically) — a 16px checkerboard of teal and white, so both colors
must appear in the window content. Then press Right: the displayed image
must change (different content pixels) and the serial must advance to the
next file.

Viewer window (wm.rs): (220, 80), content 560x460, BORDER 2, TITLE_H 22.
Without a NIC the Chat launcher is hidden, so Viewer is taskbar idx 5:
x = 70 + 5*78 + 36 = 496.
"""
import sys

from guilib import Driver, check, finish

VIEWER_BTN = (496, 768 - 20)
WIN_X, WIN_Y, CW, CH = 220, 80, 560, 460
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22

CHECK_WHITE = (240, 240, 240)   # mksite checkers_pixel
CHECK_TEAL = (20, 120, 110)


def content_region(img):
    rows = []
    for y in range(CONTENT_Y, CONTENT_Y + CH):
        i = (y * img.w + CONTENT_X) * 3
        rows.append(img.px[i:i + CW * 3])
    return b"".join(rows)


def colors_present(img, wanted):
    """Which of the wanted RGB colors appear anywhere in the content box."""
    seen = set()
    for y in range(CONTENT_Y, CONTENT_Y + CH, 4):
        for x in range(CONTENT_X, CONTENT_X + CW, 4):
            seen.add(img.at(x, y))
    return [w for w in wanted if w in seen]


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("VIEWER_OK on serial", "VIEWER_OK" in d.serial())

    mark = len(d.serial())
    d.click(*VIEWER_BTN)
    check("viewer launched", d.wait_serial("WM: launch 'viewer'", 5, mark))
    check("first image is CHECK.PNG", d.wait_serial("VIEWER: showing CHECK.PNG 128x128", 5, mark))

    d.move(1000, 700)
    img = d.dump("m23_check")
    present = colors_present(img, [CHECK_WHITE, CHECK_TEAL])
    check("checkerboard rendered (both colors present)",
          CHECK_WHITE in present and CHECK_TEAL in present, str(present))
    first = content_region(img)

    # Right arrow -> next image (M35: the disk now has JPGs too, so the second
    # image may be DOG.JPG; just require the viewer to advance to a new file).
    mark = len(d.serial())
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "right"}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "right"}}}])
    check("right arrow advances to next file",
          d.wait_serial("VIEWER: showing ", 5, mark)
          or "VIEWER: cannot decode " in d.serial()[mark:])
    d.move(1000, 700)
    img2 = d.dump("m23_next")
    check("displayed image changed after right arrow", content_region(img2) != first)

    # Left arrow returns to CHECK.PNG.
    mark = len(d.serial())
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "left"}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "left"}}}])
    check("left arrow returns to CHECK.PNG",
          d.wait_serial("VIEWER: showing CHECK.PNG", 5, mark))

    d.quit()
    finish()


if __name__ == "__main__":
    main()
