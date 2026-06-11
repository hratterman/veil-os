"""M29 proof: the in-OS file manager. Open Files from the taskbar, confirm
the file list renders (row-0 highlight + the first filename in exact font
pixels), then click the first PNG and confirm the Viewer opens with an
image loaded (serial + a non-background center pixel).

Files window (wm.rs): (120,60) content 320x378, rows 14px. Row r text at
canvas (4,14*r) [tag] and (52,14*r) [name]; row 0 selected (white on blue).
No NIC -> Files is taskbar idx 7: x = 70 + 7*78 + 36 = 652.
"""
import re
import sys

from guilib import Driver, check, finish

FILES_BTN = (652, 768 - 20)
WIN_X, WIN_Y = 120, 60
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22       # 84
NAME_X = CONTENT_X + 56          # render draws the name at canvas x+4+48+4
ROW_H = 14
COL_W = 4 + 48 + 4 + 12 * 8 + 8   # 160; the list is laid out in columns

SEL_BG = (0x2A, 0x5A, 0x8A)
SEL_TX = (0xFF, 0xFF, 0xFF)
VIEWER_BG = (0x14, 0x18, 0x1C)


def load_font():
    glyphs = {}
    src = open("src/font.rs").read()
    for i, m in enumerate(re.finditer(r"\[((?:0x[0-9a-f]{2},?\s*){16})\]", src)):
        rows = [int(b, 16) for b in re.findall(r"0x[0-9a-f]{2}", m.group(1))]
        glyphs[chr(0x20 + i)] = rows
    return glyphs


GLYPHS = load_font()


def render_expected(text, fg, bg, nrows):
    out = bytearray()
    for row in range(nrows):
        for ch in text:
            bits = GLYPHS.get(ch, GLYPHS[" "])[row]
            for n in range(8):
                out += bytes(fg if bits >> n & 1 else bg)
    return bytes(out)


def strip(img, x, y, width_px, nrows):
    out = bytearray()
    for yy in range(y, y + nrows):
        i = (yy * img.w + x) * 3
        out += img.px[i:i + width_px * 3]
    return bytes(out)


def files_listing(d):
    """Parse FILES[i]: NAME lines from serial into an ordered list."""
    rows = {}
    for m in re.finditer(r"FILES\[(\d+)\]: (\S+)", d.serial()):
        rows[int(m.group(1))] = m.group(2)
    return [rows[i] for i in sorted(rows)]


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("desktop up", d.wait_serial("WM_OK", 60))

    mark = len(d.serial())
    d.click(*FILES_BTN)
    check("files window open", d.wait_serial("FILES: window open", 8, mark))

    listing = files_listing(d)
    check("file list has entries", len(listing) > 0, f"{len(listing)} files")

    # Row 0 is selected -> a blue highlight fills its column (COL_W wide)
    # past the filename. Sample inside column 0's highlight, clear of glyphs.
    d.move(1000, 740)
    img = d.dump("m29_list")
    far = (CONTENT_X + COL_W - 12, CONTENT_Y + 7)
    check("row 0 highlighted (list rendered)", img.at(*far) == SEL_BG,
          f"px {img.at(*far)} want {SEL_BG}")

    # First filename in exact font pixels (white on blue, top 13 rows to
    # avoid the 14px row overlap at the glyph's bottom).
    first = listing[0]
    got = strip(img, NAME_X, CONTENT_Y, len(first) * 8, 13)
    want = render_expected(first, SEL_TX, SEL_BG, 13)
    check(f"first filename '{first}' rendered", got == want,
          "" if got == want else f"{sum(a != b for a, b in zip(got, want))} bytes differ")

    # Click the first PNG -> Viewer opens with that image.
    png = next((f for f in listing if f.endswith(".PNG")), None)
    check("a PNG exists on disk", png is not None, str(png))
    row = listing.index(png)
    mark = len(d.serial())
    d.click(200, CONTENT_Y + row * ROW_H + 7)
    check("PNG opened in Viewer", d.wait_serial(f"FILES: open {png} in Viewer", 8, mark))
    check("Viewer decoded the image", d.wait_serial(f"VIEWER: showing {png}", 8, mark))

    d.move(1000, 740)
    img = d.dump("m29_viewer")
    center = img.at(500, 322)
    check("Viewer shows image (not blank letterbox)", center != VIEWER_BG,
          f"center px {center}")

    d.quit()
    finish()


if __name__ == "__main__":
    main()
