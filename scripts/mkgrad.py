#!/usr/bin/env python3
"""Generate two pixel-identical PNGs — one non-interlaced (GRAD.PNG), one
Adam7-interlaced (GRADI.PNG) — to exercise the kernel PNG decoder's Adam7
path (M32-C). The decoder must produce the same image from both; the on-OS
viewer + drive_m32_interlace.py compares them pixel-for-pixel and the decoder
emits INTERLACE_OK when it sees the interlaced one.

Pure stdlib (zlib + struct), truecolor 8-bit RGB, filter-none scanlines.
"""
import struct
import sys
import zlib

W, H = 96, 96

# Adam7 pass schedule (RFC 2083 §8.2): (x_start, y_start, x_step, y_step).
ADAM7 = [
    (0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4),
    (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2),
]


def pixel(x, y):
    # A smooth 2-axis gradient with a little structure so a mis-tiled pass
    # would show up as visible banding (and as differing bytes in the proof).
    return (x * 255 // (W - 1), y * 255 // (H - 1), (x ^ y) & 0xff)


def chunk(tag, data):
    c = tag + data
    return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))


def png(width, height, interlace):
    if not interlace:
        raw = bytearray()
        for y in range(height):
            raw.append(0)  # filter: none
            for x in range(width):
                raw += bytes(pixel(x, y))
    else:
        raw = bytearray()
        for (xs, ys, xstep, ystep) in ADAM7:
            pw = (width - xs + xstep - 1) // xstep if width > xs else 0
            ph = (height - ys + ystep - 1) // ystep if height > ys else 0
            if pw == 0 or ph == 0:
                continue
            for py in range(ph):
                raw.append(0)  # filter: none
                for px in range(pw):
                    raw += bytes(pixel(xs + px * xstep, ys + py * ystep))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 1 if interlace else 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    with open(f"{out}/GRAD.PNG", "wb") as f:
        f.write(png(W, H, False))
    with open(f"{out}/GRADI.PNG", "wb") as f:
        f.write(png(W, H, True))
    print(f"wrote {out}/GRAD.PNG + {out}/GRADI.PNG ({W}x{H} RGB, plain + Adam7)")


if __name__ == "__main__":
    main()
