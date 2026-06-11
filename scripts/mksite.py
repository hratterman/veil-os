#!/usr/bin/env python3
"""Generate the hand-authored site Veil's HTTP server serves (M15) and the
on-OS browser renders (M16). Pure-stdlib PNG + GIF generation (zlib + crc32
for PNG, a from-scratch LZW encoder for the animated GIF).

HTML stays inside the M16 browser's documented subset: html/head/body,
h1-h6, p, a, ul/ol/li, img, div/span, br, pre. CSS: tag/.class selectors,
color, background-color, font-size, margin, padding, width, display.
"""
import math
import os
import struct
import sys
import zlib

OUT = os.path.join(os.path.dirname(__file__), "..", "site")

# Shared chrome ------------------------------------------------------------

NAV = (
    '<div class="nav">'
    '<a href="index.htm">Home</a> | '
    '<a href="news.htm">News</a> | '
    '<a href="wiki.htm">Wiki</a> | '
    '<a href="gallery.htm">Gallery</a> | '
    '<a href="ascii.htm">ASCII</a> | '
    '<a href="tips.htm">Tips</a> | '
    '<a href="changes.htm">Changelog</a> | '
    '<a href="about.htm">About</a> | '
    '<a href="web.htm">Web</a>'
    '</div>'
)


def page(body):
    return (
        '<html>\n<head><link rel="stylesheet" href="style.css"></head>\n'
        '<body>\n' + NAV + '\n' + body + '\n</body>\n</html>\n'
    )


STYLE = """body { background-color: #14181c; color: #d0d8e0; margin: 20px; }
h1 { color: #6cb0ff; }
h2 { color: #88b0f0; }
h3 { color: #a8c0e0; }
a { color: #e0a040; }
pre { background-color: #1b222a; color: #78e0a0; padding: 10px; }
li { color: #c8d2dc; }
div { margin: 6px; }
.nav { background-color: #1b222a; padding: 8px; }
.card { background-color: #1a2026; padding: 12px; margin: 12px; }
.hero { padding: 8px; }
.tag { color: #60c8a0; }
.muted { color: #76828e; }
.byline { color: #9aa8b8; }
.big { font-size: 22px; }
"""

# Pages --------------------------------------------------------------------

INDEX = page("""<div class="hero">
<img src="logo.png">
<h1>Veil OS</h1>
<p class="big">A graphical operating system written from scratch, and the
little internet it serves to itself.</p>
<p>Everything below this line was made by hand: the kernel, the TCP/IP
stack, the HTTP server, the FAT16 driver, the browser rendering this page,
and the PNG and GIF decoders behind its pictures.</p>
<p>More: <a href="page2.htm">how this was built</a></p>
</div>
<div class="card">
<h2>Explore the Veilnet</h2>
<ul>
<li><a href="news.htm">The Daily Veil</a> - dispatches from a world where
operating systems are current events</li>
<li><a href="wiki.htm">Veilpedia</a> - how AArch64 memory paging actually
works</li>
<li><a href="gallery.htm">Gallery</a> - images decoded by our own PNG reader</li>
<li><a href="ascii.htm">ASCII Lounge</a> - art that needs no decoder</li>
<li><a href="tips.htm">Field Guide</a> - every keyboard shortcut in the OS</li>
<li><a href="changes.htm">Changelog</a> - thirty-one milestones, gated</li>
<li><a href="about.htm">About</a> - who, what, and why</li>
<li><a href="page2.htm">The build, in order</a> - the full milestone ladder</li>
</ul>
</div>
<div class="card">
<h2>What you are touching right now</h2>
<ul>
<li>a virtio-net driver moving raw ethernet frames</li>
<li>ARP, IPv4 and ICMP - it answers ping</li>
<li>a hand-written TCP state machine; the handshake that delivered this
page was ours on one side</li>
<li>an HTTP/1.1 server running as a preemptively scheduled kernel task</li>
<li>a browser parsing our own HTML and CSS, with our own PNG and GIF
decoders for the pictures</li>
</ul>
</div>
<pre>BOOT_OK: veil kernel alive</pre>""")

NEWS = page("""<h1>The Daily Veil</h1>
<p class="byline">All the milestones fit to print | from-scratch desk</p>
<div class="card">
<h2>Operating System Declares Independence From Linux, Everything Else</h2>
<p class="byline">QEMU dateline</p>
<p>In a move analysts are calling "either heroic or deeply unnecessary," a
graphical OS booted this morning carrying nothing it did not write itself.
"No borrowed kernel, no borrowed network library, no borrowed font," a
spokesprocess said, before being preempted by the scheduler mid-sentence.</p>
</div>
<div class="card">
<h2>Local Kernel Resolves pool.ntp.org, Discovers It Is Late</h2>
<p>Veil OS sent a single NTP packet across the real internet and set its
clock to within a second of true. Sources confirm the clock had previously
believed the year was zero and the time was "since boot."</p>
</div>
<div class="card">
<h2>Two From-Scratch Operating Systems Spotted Chatting Over UDP</h2>
<p>Witnesses report two completely independent Veil instances exchanging
messages across a relay, "as if neither of them was Linux." Neither could
be reached for comment; both were busy not borrowing a network stack.</p>
</div>
<div class="card">
<h2>GIF Plays Inside Hand-Written OS; Crowd Goes Mild</h2>
<p>A user uploaded an animated GIF to a browser-playable demo and watched
it loop inside an operating system with no codec library of any kind. The
LZW decompressor, eyewitnesses say, was "about a hundred and fifty lines
and held together by spite."</p>
</div>
<p><a href="index.htm">Back to the front page</a></p>""")

WIKI = page("""<h1>Veilpedia: Virtual Memory on AArch64</h1>
<p class="muted">From Veilpedia, the encyclopedia anyone who wrote the
kernel can edit.</p>
<div class="card">
<p><span class="tag">Virtual memory</span> is the trick that lets every
program believe it owns a vast, private, flat expanse of addresses while
the hardware quietly maps those addresses onto whatever physical RAM
happens to be free. On 64-bit ARM (AArch64) the translation is done by the
Memory Management Unit walking a tree of page tables.</p>
</div>
<h2>The translation walk</h2>
<p>Veil uses a 39-bit virtual address space split into three levels of
4 KiB tables. An address is sliced into three 9-bit indices plus a 12-bit
page offset:</p>
<pre>  bits 38..30   level 1 index   (1 GiB blocks)
  bits 29..21   level 2 index   (2 MiB blocks)
  bits 20..12   level 3 index   (4 KiB pages)
  bits 11..0    offset within the page</pre>
<p>Each level holds 512 descriptors. The walk starts at the table pointed
to by <span class="tag">TTBR0_EL1</span>, indexes down three levels, and
arrives at a physical frame. A descriptor can also be a "block" entry that
stops the walk early, mapping a whole 1 GiB or 2 MiB region at once - which
is how Veil identity-maps device memory and RAM cheaply at boot.</p>
<h2>Permissions and faults</h2>
<p>Every descriptor carries access bits. User pages are marked
<span class="tag">AP_EL0</span> with execute-never on data; if code at EL0
touches kernel memory, the walk denies it and the CPU raises a translation
fault, which Veil turns into a clean process kill. This was proven by a
deliberately evil test program that read kernel memory and was, correctly,
executed - in the capital-punishment sense.</p>
<h2>See also</h2>
<ul>
<li><a href="changes.htm">Milestone 3: paging and the MMU</a></li>
<li><a href="page2.htm">The full build ladder</a></li>
</ul>
<p><a href="index.htm">Home</a></p>""")

GALLERY = page("""<h1>Gallery</h1>
<p>Every image below was decoded by Veil's own PNG reader (inflate, the
Paeth filter, and all) and drawn by the browser. The animated one is
decoded by our from-scratch GIF89a / LZW player - open it in the
<span class="tag">GIF</span> app on the desktop.</p>
<div class="card">
<img src="logo.png">
<p>logo.png - a hand-carved "veil" gradient, 64x64</p>
</div>
<div class="card">
<img src="sunset.png">
<p>sunset.png - a math sunset, 128x128, generated pixel by pixel</p>
</div>
<div class="card">
<img src="plasma.png">
<p>plasma.png - three sine waves summed into a field</p>
</div>
<div class="card">
<img src="check.png">
<p>check.png - the humble checkerboard, useful for proving a decoder is
honest</p>
</div>
<p><a href="index.htm">Home</a></p>""")

ASCII = page("""<h1>The ASCII Lounge</h1>
<p>No decoder required - the browser draws these from a bitmap font it was
handed at boot.</p>
<pre>
        .-----------------------------.
        |  V E I L   O S              |
        |  a from-scratch aarch64 os  |
        '-----------------------------'
               |   |
            .--' '--.
            |  o o  |     hello from ring 0
            |   ^   |
            |  '-'  |
            '-------'
</pre>
<pre>
   /\\_/\\     paged,
  ( o.o )    preempted,
   > ^ <     and proud
</pre>
<pre>
  [boot]->[mmu]->[heap]->[sched]->[fs]->[net]->[gui]
     reality is the grader, and reality passed us
</pre>
<p><a href="index.htm">Home</a></p>""")

TIPS = page("""<h1>Field Guide</h1>
<p>Veil opens nothing on boot. Launch apps from the bottom taskbar or the
desktop icon grid - both open-or-raise. Every title bar has a close zone at
its right edge. Here is what the keys do once a window has focus.</p>
<div class="card">
<h3>Editor</h3>
<ul>
<li>type to insert, Backspace to delete, Enter for newlines</li>
<li>SAV writes the file to the FAT16 disk, LOD re-reads it</li>
</ul>
</div>
<div class="card">
<h3>Image viewer and GIF player</h3>
<ul>
<li>Left / Right arrows: previous / next</li>
<li>GIF player: Space toggles play and pause; Up / Down switch files</li>
</ul>
</div>
<div class="card">
<h3>Clock</h3>
<ul>
<li>click the face to cycle: wall, digital, chronograph, stopwatch</li>
<li>STA / STP / RST drive the chronograph and stopwatch</li>
</ul>
</div>
<div class="card">
<h3>Shell</h3>
<ul>
<li><span class="tag">ls</span>, <span class="tag">cat</span>,
<span class="tag">echo</span>, <span class="tag">spin</span>,
<span class="tag">paint</span>, <span class="tag">help</span></li>
<li>user programs run at EL0 and are preemptively scheduled</li>
</ul>
</div>
<p><a href="index.htm">Home</a></p>""")

ABOUT = page("""<h1>About Veil OS</h1>
<div class="card">
<p>Veil is a graphical operating system for 64-bit ARM, built milestone by
milestone with a single rule: <span class="tag">reality is the grader</span>.
Nothing counted as done until it provably worked in QEMU - pixels on a
screen, packets on the wire, files surviving a reboot.</p>
</div>
<h2>The principle</h2>
<p>No Linux. No BSD. No borrowed kernel, network library, font engine, or
codec. When the build needed an HTTP server, we wrote one. When it needed a
browser, we wrote that too, and a PNG decoder, and now a GIF decoder. The
point was never to be fast or complete; it was to be honest.</p>
<h2>Colophon</h2>
<ul>
<li>language: Rust, no_std, on bare metal</li>
<li>targets: QEMU virt (aarch64) and a real Raspberry Pi 4</li>
<li>font: an 8x16 bitmap, doubled from a public-domain 8x8</li>
<li>this page: served by the OS to its own browser over loopback TCP</li>
</ul>
<p>Source and a one-line installer live on the
<a href="changes.htm">changelog</a> page.</p>
<p><a href="index.htm">Home</a></p>""")

WEB = page("""<h1>The Real Web</h1>
<p><a href="imgtest.htm">External images test</a> -
<a href="cssvar.htm">CSS variables test</a> -
<a href="flextest.htm">Flexbox test</a></p>
<div class="card">
<p>These links leave the island. The browser hands the full URL to a small
host-side proxy that fetches the real (often HTTPS) page, strips it to what
this renderer understands, and sends back plain HTML over our own TCP. No
JavaScript, no images - just the text of the live internet, in a browser
written from scratch.</p>
</div>
<h2>Try these</h2>
<ul>
<li><a href="http://neverssl.com">neverssl.com</a> - plain HTTP, always up</li>
<li><a href="http://example.com">example.com</a> - the minimal classic</li>
<li><a href="https://example.com">example.com (HTTPS)</a> - direct TLS 1.3, no proxy</li>
<li><a href="https://lite.cnn.com">lite.cnn.com</a> - CNN, text-only edition</li>
<li><a href="https://en.wikipedia.org/wiki/ARM_architecture_family">Wikipedia: ARM</a> - text-heavy, renders well</li>
<li><a href="https://news.ycombinator.com">Hacker News</a> - simple HTML</li>
</ul>
<p class="muted">Slow to load is normal - the proxy is fetching a real site
across the internet, then this browser lays out every line by hand.</p>
<p><a href="index.htm">Home</a></p>""")

FLEXTEST = page("""<style>
.fnav { display: flex; flex-direction: row; justify-content: space-between; align-items: center; background-color: #1a2e1a; padding: 12px; gap: 8px; }
.fnav a { color: #f5f0e8; }
.cards { display: flex; flex-direction: row; flex-wrap: wrap; gap: 14px; }
.card { flex: 1; background-color: #243024; color: #ffffff; padding: 12px; }
</style>
<div class="fnav">
<a href="flexhome.htm">HOME</a>
<a href="flexwork.htm">WORK</a>
<a href="flexabout.htm">ABOUT</a>
<a href="flexcontact.htm">CONTACT</a>
</div>
<h2>Cards</h2>
<div class="cards">
<div class="card">Card one body text here.</div>
<div class="card">Card two body text here.</div>
<div class="card">Card three body text here.</div>
</div>
<p><a href="web.htm">Back</a></p>""")

CSSVAR = page("""<style>
:root { --brand: #2a7e3b; --accent: #cc4422; --pad: 14px; }
.vbar { background-color: var(--brand); color: #ffffff; padding: var(--pad); }
.vtext { color: var(--accent); font-size: 32px; }
.vfb { color: var(--nope, #1188dd); }
</style>
<div class="vbar">brand colored bar (background = var(--brand))</div>
<h1 class="vtext">accent heading (color = var(--accent))</h1>
<p class="vfb">fallback works (var(--nope, #1188dd))</p>
<p><a href="web.htm">Back</a></p>""")

IMGTEST = page("""<h1>External images</h1>
<p>A PNG fetched over direct TLS 1.3 (no proxy):</p>
<img src="https://www.python.org/static/img/python-logo.png">
<p>A PNG fetched over plain HTTP through the proxy:</p>
<img src="http://www.gnu.org/graphics/heckert_gnu.small.png">
<p><a href="web.htm">Back</a></p>""")

CHANGES = page("""<h1>Changelog</h1>
<p class="muted">Every milestone gated behind an observed proof before the
next began. The phases, at a glance:</p>
<table>
<tr><th>Phase</th><th>Milestones</th><th>Highlight</th></tr>
<tr><td>Boot to OS</td><td>M1 - M15</td><td>its own TCP/IP stack</td></tr>
<tr><td>A computer</td><td>M16 - M24</td><td>browser, audio, real Pi 4</td></tr>
<tr><td>Hosted demo</td><td>M25 - M31</td><td>uploads and a GIF player</td></tr>
<tr><td>Overnight</td><td>M32</td><td>scroll, history, tables, Lisp, the internet</td></tr>
</table>
<div class="card">
<h2>M31 - Internet expansion and the GIF player <span class="tag">new</span></h2>
<p>This little web you are browsing grew from two pages to a small
internet, and the OS learned to decode animated GIFs from scratch - LZW and
all - so you can upload one and watch it loop inside a from-scratch kernel.</p>
</div>
<div class="card">
<h2>M25 to M30 - The hosted demo</h2>
<p>Per-visitor sessions, a TCP chat relay with direct messages, a first-boot
setup screen, browser audio over WebSocket, an in-OS file manager, and a
pre-boot upload page so visitors can bring their own images, sounds, and
now GIFs.</p>
</div>
<div class="card">
<h2>M16 to M24 - Becoming a computer</h2>
<p>An on-OS browser, a real Raspberry Pi 4 port, a persistent text editor,
a ticking clock with NTP-synced local time, global chat over UDP, an image
viewer, and native WAV audio.</p>
</div>
<div class="card">
<h2>M1 to M15 - Becoming an OS</h2>
<p>Serial boot, exceptions and a timer, paging and the MMU, a heap, a
framebuffer, input, a window manager, paint, user mode, a disk, a shell,
and a from-scratch TCP/IP stack ending in this very HTTP server.</p>
</div>
<h3>Run it yourself</h3>
<pre>brew install qemu &amp;&amp; git clone https://github.com/hratterman/veil-os &amp;&amp; cd veil-os &amp;&amp; scripts/demo.sh</pre>
<p><a href="page2.htm">See the full ordered ladder</a> | <a href="index.htm">Home</a></p>""")

PAGE2 = page("""<h1>The milestone ladder</h1>
<p>Veil was built strictly gated: each milestone proven in QEMU before the
next began.</p>
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
<li>an on-OS browser - html, css, png, all ours</li>
<li>the same kernel on a real Raspberry Pi 4</li>
<li>a text editor with persistent files</li>
<li>a clock with NTP-synced real local time</li>
<li>global chat between independent Veil instances over UDP</li>
<li>a public GitHub release and a browser-playable hosted demo</li>
<li>a PNG image viewer</li>
<li>a native WAV audio player</li>
<li>per-visitor sessions, chat relay, setup screen, browser audio</li>
<li>an in-OS file manager and a pre-boot upload page</li>
<li>this expanded internet and a from-scratch animated GIF player</li>
</ol>
<p><a href="index.htm">back home</a></p>""")

# Generated images ---------------------------------------------------------


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
    in_v = abs(x - 32) < (y - 8) // 2 + 2 and 8 <= y < 52 and abs(x - 32) > (y - 8) // 2 - 6
    if in_v:
        return (224, 160, 64)
    r = 24 + x // 3
    g = 40 + y // 2
    b = 120 + (x + y) // 2
    return (r, g, min(b, 255))


def sunset_pixel(x, y):
    W, H = 128, 128
    cx, cy, r = W // 2, H // 3, 18
    if (x - cx) ** 2 + (y - cy) ** 2 < r * r:
        return (255, 240, 60)
    t = y / H
    red = int(220 * (1 - t) + 10 * t)
    green = int(120 * (1 - t) + 20 * t)
    blue = int(40 * (1 - t) + 80 * t)
    return (red, green, blue)


def checkers_pixel(x, y):
    cell = 16
    if (x // cell + y // cell) % 2 == 0:
        return (240, 240, 240)
    return (20, 120, 110)


def plasma_pixel(x, y):
    v = math.sin(x / 8.0) + math.sin(y / 8.0) + math.sin((x + y) / 12.0)
    t = int((v + 3) / 6 * 255)
    return (t, (t * 2) % 256, 255 - t)


# Animated GIF (from-scratch GIF89a + LZW encoder, stdlib only) ------------


def gif_lzw(indices, min_code_size):
    """GIF-flavour LZW: variable-width codes, LSB-first bit packing, with a
    clear code and end-of-information code. Standard giflib-compatible
    code-size bumping so any conformant decoder (incl. ours) reads it."""
    clear = 1 << min_code_size
    eoi = clear + 1
    out = bytearray()
    bitbuf = 0
    bitcnt = 0

    def emit(code, size):
        nonlocal bitbuf, bitcnt
        bitbuf |= code << bitcnt
        bitcnt += size
        while bitcnt >= 8:
            out.append(bitbuf & 0xFF)
            bitbuf >>= 8
            bitcnt -= 8

    def fresh():
        return ({bytes([i]): i for i in range(clear)}, clear + 2, min_code_size + 1)

    table, next_code, code_size = fresh()
    emit(clear, code_size)
    buf = bytes([indices[0]])
    for idx in indices[1:]:
        nb = buf + bytes([idx])
        if nb in table:
            buf = nb
            continue
        emit(table[buf], code_size)
        if next_code < 4096:
            table[nb] = next_code
            next_code += 1
            # Bump one code later than the table size (next_code == 2^cs + 1):
            # the decoder adds entries one step behind, so this keeps the two
            # in lock-step. (Verified by round-trip against a standard decoder.)
            if next_code == (1 << code_size) + 1 and code_size < 12:
                code_size += 1
        else:
            emit(clear, code_size)
            table, next_code, code_size = fresh()
        buf = bytes([idx])
    emit(table[buf], code_size)
    emit(eoi, code_size)
    if bitcnt > 0:
        out.append(bitbuf & 0xFF)
    return bytes(out)


def gif_sub_blocks(data):
    """Wrap LZW bytes into <=255-byte sub-blocks, 0x00-terminated."""
    out = bytearray()
    for off in range(0, len(data), 255):
        chunk = data[off:off + 255]
        out.append(len(chunk))
        out += chunk
    out.append(0)
    return bytes(out)


def gif_palette():
    """256 entries: a smooth rainbow, index 255 reserved white."""
    pal = []
    for i in range(255):
        a = i / 255 * 2 * math.pi
        r = int(128 + 110 * math.sin(a))
        g = int(128 + 110 * math.sin(a + 2.094))
        b = int(128 + 110 * math.sin(a + 4.188))
        pal.append((max(0, min(255, r)), max(0, min(255, g)), max(0, min(255, b))))
    pal.append((255, 255, 255))
    return pal


def gif_frames(w, h, n):
    """Plasma field with a white bouncing ball — clear, looping motion."""
    frames = []
    for f in range(n):
        ph = f / n * 2 * math.pi
        bx = w // 2 + int((w // 2 - 6) * math.sin(ph * 2))
        by = h // 2 + int((h // 2 - 6) * math.cos(ph * 3))
        idx = bytearray(w * h)
        p = 0
        for y in range(h):
            for x in range(w):
                if (x - bx) ** 2 + (y - by) ** 2 < 16:
                    idx[p] = 255
                else:
                    v = (math.sin(x / 8.0 + ph) + math.sin(y / 8.0 - ph)
                         + math.sin((x + y) / 10.0 + ph * 2))
                    idx[p] = int((v + 3) / 6 * 253) % 254
                p += 1
        frames.append(bytes(idx))
    return frames


def write_demo_gif(path, w=64, h=64, n=12, delay_cs=8):
    pal = gif_palette()
    out = bytearray()
    out += b"GIF89a"
    out += struct.pack("<HH", w, h)
    out += bytes([0xF7, 0, 0])            # GCT 256, color res 8, bg 0, aspect 0
    for (r, g, b) in pal:
        out += bytes([r, g, b])
    # NETSCAPE loop-forever extension.
    out += bytes([0x21, 0xFF, 0x0B]) + b"NETSCAPE2.0"
    out += bytes([0x03, 0x01, 0x00, 0x00, 0x00])
    for frame in gif_frames(w, h, n):
        # Graphic control extension: disposal 1 (leave), delay, no transparency.
        out += bytes([0x21, 0xF9, 0x04, 0x04]) + struct.pack("<H", delay_cs) + bytes([0, 0])
        # Image descriptor: full canvas, no local table, not interlaced.
        out += bytes([0x2C]) + struct.pack("<HHHH", 0, 0, w, h) + bytes([0x00])
        out += bytes([8])                  # LZW minimum code size
        out += gif_sub_blocks(gif_lzw(frame, 8))
    out += bytes([0x3B])                   # trailer
    with open(path, "wb") as f:
        f.write(out)
    print(f"demo.gif: {w}x{h}, {n} frames, {len(out)} bytes")


def main():
    os.makedirs(OUT, exist_ok=True)
    pages = {
        "index.htm": INDEX, "page2.htm": PAGE2, "news.htm": NEWS,
        "wiki.htm": WIKI, "gallery.htm": GALLERY, "ascii.htm": ASCII,
        "tips.htm": TIPS, "about.htm": ABOUT, "changes.htm": CHANGES,
        "web.htm": WEB, "imgtest.htm": IMGTEST, "cssvar.htm": CSSVAR, "flextest.htm": FLEXTEST,
        "style.css": STYLE,
    }
    for name, text in pages.items():
        with open(os.path.join(OUT, name), "w") as f:
            f.write(text)
    with open(os.path.join(OUT, "logo.png"), "wb") as f:
        f.write(png(64, 64, logo_pixel))
    with open(os.path.join(OUT, "sunset.png"), "wb") as f:
        f.write(png(128, 128, sunset_pixel))
    with open(os.path.join(OUT, "check.png"), "wb") as f:
        f.write(png(128, 128, checkers_pixel))
    with open(os.path.join(OUT, "plasma.png"), "wb") as f:
        f.write(png(128, 128, plasma_pixel))
    write_demo_gif(os.path.join(OUT, "demo.gif"))
    print("site/ ready:", ", ".join(sorted(os.listdir(OUT))))


if __name__ == "__main__":
    sys.exit(main())
