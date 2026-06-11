"""M30 proof driver: a session-manager-spawned instance whose disk was
built with an uploaded PNG. Complete the first-boot setup, open Files, and
verify the uploaded filename (VEILTEST.PNG) appears in the list (exact font
pixels) and opens in the Viewer.

Files window (wm.rs): (120,60) content 320x378, rows 14px; name at canvas
x=52 (screen 174). With a NIC the Files launcher is taskbar idx 8:
x = 70 + 8*78 + 36 = 730.
"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

FILES_BTN = (730, 768 - 20)
CONTENT_X = 120 + 2
CONTENT_Y = 60 + 2 + 22       # 84
NAME_X = CONTENT_X + 52       # 174
ROW_H = 14
UPLOAD = "VEILTEST.PNG"

BG = (0x14, 0x1A, 0x22)
TEXT = (0xD0, 0xD8, 0xE0)
SEL_BG = (0x2A, 0x5A, 0x8A)
VIEWER_BG = (0x14, 0x18, 0x1C)


def load_font():
    glyphs = {}
    src = open("src/font.rs").read()
    for i, m in enumerate(re.finditer(r"\[((?:0x[0-9a-f]{2},?\s*){16})\]", src)):
        rows = [int(b, 16) for b in re.findall(r"0x[0-9a-f]{2}", m.group(1))]
        glyphs[chr(0x20 + i)] = rows
    return glyphs


GLYPHS = load_font()


def expected(text, fg, bg, r0, r1):
    out = bytearray()
    for row in range(r0, r1):
        for ch in text:
            bits = GLYPHS.get(ch, GLYPHS[" "])[row]
            for n in range(8):
                out += bytes(fg if bits >> n & 1 else bg)
    return bytes(out)


def strip(img, x, y, width_px, r0, r1):
    out = bytearray()
    for yy in range(y + r0, y + r1):
        i = (yy * img.w + x) * 3
        out += img.px[i:i + width_px * 3]
    return bytes(out)


def listing(d):
    rows = {}
    for m in re.finditer(r"FILES\[(\d+)\]: (\S+)", d.serial()):
        rows[int(m.group(1))] = m.group(2)
    return [rows[i] for i in sorted(rows)]


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])

    # Fresh hosted disk -> first-boot setup screen. Name ourselves and enter.
    check("setup screen shown", d.wait_serial("SETUP: first boot", 60))
    d.type_text("demo")
    d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "ret"}}}])
    d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "ret"}}}])
    check("desktop reached", d.wait_serial("WM_OK", 20))

    mark = len(d.serial())
    d.click(*taskbar_xy(d, "files"))
    check("files window open", d.wait_serial("FILES: window open", 8, mark))

    names = listing(d)
    check("uploaded file is on disk + listed", UPLOAD in names,
          f"{UPLOAD} in {names}")
    row = names.index(UPLOAD)

    d.move(1000, 740)
    img = d.dump("m30_list")
    # List rendered (row-0 highlight to the right of the filename).
    far = (CONTENT_X + 300, CONTENT_Y + 7)
    check("file list rendered", img.at(*far) == SEL_BG, f"px {img.at(*far)}")
    # Uploaded filename in exact font pixels (unselected row, rows 2..13 to
    # dodge the 14px inter-row overlap).
    got = strip(img, NAME_X, CONTENT_Y + row * ROW_H, len(UPLOAD) * 8, 2, 14)
    want = expected(UPLOAD, TEXT, BG, 2, 14)
    check(f"'{UPLOAD}' rendered in the list", got == want,
          "" if got == want else f"{sum(a != b for a, b in zip(got, want))} bytes differ")

    # Open it -> Viewer shows the uploaded image.
    mark = len(d.serial())
    d.click(200, CONTENT_Y + row * ROW_H + 7)
    check("uploaded PNG opens in Viewer",
          d.wait_serial(f"VIEWER: showing {UPLOAD}", 8, mark))
    d.move(1000, 740)
    img = d.dump("m30_viewer")
    check("Viewer shows the image", img.at(500, 322) != VIEWER_BG,
          f"center px {img.at(500, 322)}")

    d.quit()
    finish()


if __name__ == "__main__":
    main()
