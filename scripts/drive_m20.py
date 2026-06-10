"""M20 proof: two Veil instances chat over their own UDP stacks.

Instance A types a message; it must appear in instance B's chat window
(and vice versa). The pixel check is exact: the expected log line is
rendered host-side from the kernel's own font table (src/font.rs) and
compared byte-for-byte against the captured strip — own messages in
CHAT_MINE blue, the peer's in CHAT_TEXT ink, both over the CHAT_BG paper.

Chat geometry (desktop.rs / wm.rs): window at (40, 380), content 440x300;
log row i is drawn at canvas (6, 4 + 16*i).
"""
import re
import sys

from guilib import Driver, check, finish

WIN_X, WIN_Y, CW = 40, 380, 440
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22
ROW_X = CONTENT_X + 6
ROW_Y = CONTENT_Y + 4

CHAT_BG = (0xF8, 0xF6, 0xF0)
CHAT_TEXT = (0x20, 0x28, 0x30)
CHAT_MINE = (0x20, 0x60, 0xA0)

MSG_A = "hello from a over our own udp"
MSG_B = "hi back from b same stack"


def load_font():
    glyphs = {}
    src = open("src/font.rs").read()
    for i, m in enumerate(re.finditer(r"\[((?:0x[0-9a-f]{2},?\s*){16})\]", src)):
        rows = [int(b, 16) for b in re.findall(r"0x[0-9a-f]{2}", m.group(1))]
        glyphs[chr(0x20 + i)] = rows
    assert len(glyphs) == 95, f"parsed {len(glyphs)} glyphs"
    return glyphs


GLYPHS = load_font()


def render_expected(text, fg):
    """The strip draw_string would produce: fg where a glyph bit is set,
    CHAT_BG elsewhere. Bit n of a row byte lights pixel x = n."""
    w = len(text) * 8
    out = bytearray()
    for row in range(16):
        for ch in text:
            bits = GLYPHS.get(ch, GLYPHS[" "])[row]
            for n in range(8):
                out += bytes(fg if bits >> n & 1 else CHAT_BG)
    assert len(out) == w * 16 * 3
    return bytes(out)


def captured_strip(img, row, width_px):
    out = bytearray()
    y0 = ROW_Y + 16 * row
    for y in range(y0, y0 + 16):
        i = (y * img.w + ROW_X) * 3
        out += img.px[i:i + width_px * 3]
    return bytes(out)


def check_line(d, shot, row, text, fg, label):
    d.move(1000, 740)
    img = d.dump(shot)
    got = captured_strip(img, row, len(text) * 8)
    want = render_expected(text, fg)
    diff = sum(a != b for a, b in zip(got, want))
    check(label, got == want, "" if got == want else f"{diff} bytes differ")


def main():
    qmp_a, ser_a, qmp_b, ser_b, shots = sys.argv[1:6]
    a = Driver(qmp_a, ser_a, shots)
    b = Driver(qmp_b, ser_b, shots)

    # UX overhaul: nothing opens at boot. Wait for each desktop to be live
    # (the offline bridge has no NTP, so boot is delayed by the sync
    # timeout), then launch Chat from the taskbar (idx 5 with a NIC
    # present: x = 70 + 5*78 + 36 = 496).
    check("A desktop up", a.wait_serial("WM_OK", 40))
    check("B desktop up", b.wait_serial("WM_OK", 40))
    a.click(496, 768 - 20)
    b.click(496, 768 - 20)
    check("A chat window open", a.wait_serial("CHAT: window open as 'A'", 30))
    check("B chat window open", b.wait_serial("CHAT: window open as 'B'", 30))

    # --- A -> B --------------------------------------------------------
    mark_a, mark_b = len(a.serial()), len(b.serial())
    a.click(240, 500)  # make sure chat has focus in A
    a.type_text(MSG_A + "\n")
    check("A sent", a.wait_serial(f"CHAT: sent {len(MSG_A) + 4} bytes: A: {MSG_A}", 10, mark_a))
    check("A emits CHAT_OK", a.wait_serial("CHAT_OK", 5))
    check("B received over the wire", b.wait_serial(f'CHAT: rx "A: {MSG_A}"', 15, mark_b))
    check("B emits CHAT_OK", b.wait_serial("CHAT_OK", 5))
    check_line(b, "m20_b_recv", 0, f"A: {MSG_A}", CHAT_TEXT,
               "B renders A's message (exact font pixels)")
    check_line(a, "m20_a_sent", 0, f"A: {MSG_A}", CHAT_MINE,
               "A renders its own message (mine color)")

    # --- B -> A --------------------------------------------------------
    mark_a, mark_b = len(a.serial()), len(b.serial())
    b.click(240, 500)
    b.type_text(MSG_B + "\n")
    check("B sent", b.wait_serial(f"CHAT: sent {len(MSG_B) + 4} bytes: B: {MSG_B}", 10, mark_b))
    check("A received over the wire", a.wait_serial(f'CHAT: rx "B: {MSG_B}"', 15, mark_a))
    check_line(a, "m20_a_recv", 1, f"B: {MSG_B}", CHAT_TEXT,
               "A renders B's message (exact font pixels)")
    check_line(b, "m20_b_sent", 1, f"B: {MSG_B}", CHAT_MINE,
               "B renders its own message (mine color)")

    a.quit()
    b.quit()
    finish()


if __name__ == "__main__":
    main()
