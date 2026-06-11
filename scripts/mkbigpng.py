#!/usr/bin/env python3
"""Write a large truecolor PNG to stress the kernel's image decoder.

A smooth gradient compresses to a small file but still decodes to a full
WxH XRGB pixel buffer plus inflated scanlines — the exact memory shape that
used to OOM-panic the OS when a real-world photo (e.g. 1920x1080) was opened.

Usage: mkbigpng.py <out.png> [width] [height]
"""
import struct
import sys
import zlib


def chunk(tag, data):
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def main():
    out = sys.argv[1]
    w = int(sys.argv[2]) if len(sys.argv) > 2 else 1920
    h = int(sys.argv[3]) if len(sys.argv) > 3 else 1080

    # Smooth ramps in every channel: a real full-resolution gradient (lots of
    # distinct colors to prove it rendered) that still compresses to a small
    # file so it fits the 16 MB demo disk.
    raw = bytearray()
    for y in range(h):
        raw.append(0)  # filter: none
        gy = (y * 255) // max(h - 1, 1)
        for x in range(w):
            raw.append((x * 255) // max(w - 1, 1))            # R ramps across
            raw.append(gy)                                    # G ramps down
            raw.append(((x + y) * 255) // max(w + h - 2, 1))  # B diagonal
    idat = zlib.compress(bytes(raw), 6)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))  # 8-bit truecolor
    png += chunk(b"IDAT", idat)
    png += chunk(b"IEND", b"")

    with open(out, "wb") as f:
        f.write(png)
    print(f"wrote {out}: {w}x{h}, {len(png)} bytes on disk")


if __name__ == "__main__":
    main()
