#!/usr/bin/env python3
"""Assert the M5 test scene actually rendered: parse the PPM screendump and
check exact colors at known coordinates (kept in sync with milestone5() in
src/main.rs). Exits non-zero on any mismatch."""
import sys


def load_ppm(path):
    data = open(path, "rb").read()
    # P6\n<w> <h>\n255\n then binary RGB. QEMU writes no comment lines.
    parts = data.split(b"\n", 3)
    assert parts[0] == b"P6", "not a P6 ppm"
    w, h = map(int, parts[1].split())
    assert parts[2] == b"255"
    px = parts[3]
    assert len(px) >= w * h * 3
    return w, h, px


def main():
    w, h, px = load_ppm(sys.argv[1])

    def at(x, y):
        i = (y * w + x) * 3
        return (px[i], px[i + 1], px[i + 2])

    checks = [
        ("background", (20, 500), (0x10, 0x20, 0x40)),
        ("red rect", (100, 100), (0xE0, 0x30, 0x30)),
        ("green rect", (250, 100), (0x30, 0xC0, 0x60)),
        ("blue rect", (400, 100), (0x30, 0x60, 0xE0)),
        ("white border", (2, 2), (0xFF, 0xFF, 0xFF)),
        ("gradient left", (0, 616), (0x00, 0x00, 0x00)),
        ("gradient right", (w - 1, 616), (0xFF, 0x7F, 0x00)),
    ]
    failed = False
    for name, (x, y), want in checks:
        got = at(x, y)
        ok = got == want
        failed |= not ok
        print(f"{'ok  ' if ok else 'FAIL'} {name:15s} @({x},{y}): got rgb{got}, want rgb{want}")

    # Text row: white glyph pixels must exist somewhere in the "VEIL OS" line.
    text_white = sum(
        1 for y in range(300, 316) for x in range(50, 120) if at(x, y) == (0xFF, 0xFF, 0xFF)
    )
    ok = text_white > 30
    failed |= not ok
    print(f"{'ok  ' if ok else 'FAIL'} font blitter: {text_white} lit glyph pixels in text row (need >30)")

    print(f"resolution: {w}x{h}")
    if failed or (w, h) != (1024, 768):
        sys.exit(1)
    print("M5 SCREEN VERIFIED")


if __name__ == "__main__":
    main()
