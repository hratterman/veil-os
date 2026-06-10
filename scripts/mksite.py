#!/usr/bin/env python3
"""Generate the hand-authored site Veil's HTTP server serves (M15) and the
on-OS browser renders (M16). Pure-stdlib PNG generation (zlib + crc32).

HTML stays inside the M16 browser's documented subset: html/head/body,
h1-h6, p, a, ul/ol/li, img, div/span, br, pre.
"""
import os
import struct
import sys
import zlib

OUT = os.path.join(os.path.dirname(__file__), "..", "site")

INDEX = """<html>
<head><link rel="stylesheet" href="style.css"></head>
<body>
<div class="hero">
<img src="logo.png">
<h1>Veil OS</h1>
<p>This page is served by an operating system written from scratch:
its own kernel, its own TCP/IP stack, its own HTTP server, off its own
FAT16 filesystem driver.</p>
</div>
<h2>What you are touching right now</h2>
<ul>
<li>a virtio-net driver moving raw ethernet frames</li>
<li>ARP, IPv4 and ICMP (it answers ping)</li>
<li>a hand-written TCP state machine - the handshake that delivered
this page was ours on one side</li>
<li>an HTTP/1.1 server running as a preemptively scheduled kernel task</li>
</ul>
<p>More: <a href="page2.htm">how this was built</a></p>
<pre>BOOT_OK: veil kernel alive</pre>
</body>
</html>
"""

PAGE2 = """<html>
<head><link rel="stylesheet" href="style.css"></head>
<body>
<h1>The milestone ladder</h1>
<p>Veil was built strictly gated: each milestone proven in QEMU before
the next began.</p>
<ol>
<li>serial boot</li>
<li>exceptions + timer</li>
<li>paging + MMU</li>
<li>kernel heap</li>
<li>ramfb framebuffer</li>
<li>virtio keyboard and tablet</li>
<li>window manager</li>
<li>paint</li>
<li>user mode + syscalls</li>
<li>virtio-blk + FAT16</li>
<li>shell + preemptive multitasking</li>
<li>raw ethernet frames</li>
<li>arp / ipv4 / icmp</li>
<li>udp and tcp</li>
<li>this http server</li>
</ol>
<p><a href="index.htm">back home</a></p>
</body>
</html>
"""

STYLE = """body { background-color: #14181c; color: #d0d8e0; margin: 24px; }
h1 { color: #6090e0; }
h2 { color: #80a8e8; }
a { color: #e0a040; }
pre { background-color: #20262c; color: #80e0a0; padding: 8px; }
div { margin: 8px; }
"""


def png(width, height, pixel_fn):
    """Minimal truecolor PNG writer."""
    raw = b""
    for y in range(height):
        raw += b"\x00"  # filter: none
        for x in range(width):
            raw += bytes(pixel_fn(x, y))

    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))

    ihdr = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def logo_pixel(x, y):
    # A "veil": blue gradient field with a dark V carved out.
    in_v = abs(x - 32) < (y - 8) // 2 + 2 and 8 <= y < 52 and abs(x - 32) > (y - 8) // 2 - 6
    if in_v:
        return (224, 160, 64)
    r = 24 + x // 3
    g = 40 + y // 2
    b = 120 + (x + y) // 2
    return (r, g, min(b, 255))


def sunset_pixel(x, y):
    """A fake sunset: orange sky fading to deep blue at the bottom, with a circle sun."""
    W, H = 128, 128
    # Sun
    cx, cy, r = W // 2, H // 3, 18
    if (x - cx) ** 2 + (y - cy) ** 2 < r * r:
        return (255, 240, 60)
    # Sky gradient: orange at top, dark blue at bottom
    t = y / H
    red   = int(220 * (1 - t) + 10 * t)
    green = int(120 * (1 - t) + 20 * t)
    blue  = int(40  * (1 - t) + 80 * t)
    return (red, green, blue)


def checkers_pixel(x, y):
    """Classic checkerboard in deep teal and white, 16x16 squares."""
    cell = 16
    if (x // cell + y // cell) % 2 == 0:
        return (240, 240, 240)
    return (20, 120, 110)


def plasma_pixel(x, y):
    """Psychedelic plasma using pure integer math."""
    import math
    v = math.sin(x / 8.0) + math.sin(y / 8.0) + math.sin((x + y) / 12.0)
    # v in [-3, 3], map to [0, 255]
    t = int((v + 3) / 6 * 255)
    r = t
    g = (t * 2) % 256
    b = (255 - t)
    return (r, g, b)


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, text in [("index.htm", INDEX), ("page2.htm", PAGE2), ("style.css", STYLE)]:
        with open(os.path.join(OUT, name), "w") as f:
            f.write(text)
    with open(os.path.join(OUT, "logo.png"), "wb") as f:
        f.write(png(64, 64, logo_pixel))
    # Sample images for the M23 image viewer
    with open(os.path.join(OUT, "sunset.png"), "wb") as f:
        f.write(png(128, 128, sunset_pixel))
    with open(os.path.join(OUT, "check.png"), "wb") as f:
        f.write(png(128, 128, checkers_pixel))
    with open(os.path.join(OUT, "plasma.png"), "wb") as f:
        f.write(png(128, 128, plasma_pixel))
    print("site/ ready:", ", ".join(sorted(os.listdir(OUT))))


if __name__ == "__main__":
    sys.exit(main())
