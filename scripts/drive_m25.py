"""M25 proof: two isolated Veil instances, each booted from its OWN disk
image carrying a different USER.TXT. The Chat app must label outgoing
messages with the on-disk username (not the old IP-derived A/B), and the
peer must render that username verbatim.

Instance A's disk has USER.TXT="alpha_fox"; B's has "beta_owl". A types a
message; it must appear in B's chat window prefixed "alpha_fox:" (exact
font pixels, rendered host-side from src/font.rs). Both serials emit
CHAT_OK.

Chat geometry (wm.rs): window at (40, 380), content 440x300; log row i is
drawn at canvas (6, 4 + 16*i).
"""
import re
import sys

from guilib import Driver, check, finish

WIN_X, WIN_Y = 40, 380
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22
ROW_X = CONTENT_X + 6
ROW_Y = CONTENT_Y + 4

CHAT_BG = (0xF8, 0xF6, 0xF0)
CHAT_TEXT = (0x20, 0x28, 0x30)
CHAT_MINE = (0x20, 0x60, 0xA0)

USER_A = "alpha_fox"
USER_B = "beta_owl"
MSG_A = "isolated instance one"


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

    check("A desktop up", a.wait_serial("WM_OK", 60))
    check("B desktop up", b.wait_serial("WM_OK", 60))
    # Chat is taskbar button index 5 (NIC present): x = 70 + 5*78 + 36 = 496.
    a.click(496, 768 - 20)
    b.click(496, 768 - 20)
    check(f"A chat opens as '{USER_A}'",
          a.wait_serial(f"CHAT: window open as '{USER_A}'", 30))
    check(f"B chat opens as '{USER_B}'",
          b.wait_serial(f"CHAT: window open as '{USER_B}'", 30))

    mark_a, mark_b = len(a.serial()), len(b.serial())
    a.click(240, 500)  # focus chat in A
    a.type_text(MSG_A + "\n")
    line = f"{USER_A}: {MSG_A}"
    check("A sent", a.wait_serial(f"CHAT: sent {len(line) + 1} bytes: {line}", 10, mark_a))
    check("A emits CHAT_OK", a.wait_serial("CHAT_OK", 5))
    check("B received over the wire", b.wait_serial(f'CHAT: rx "{line}"', 15, mark_b))
    check("B emits CHAT_OK", b.wait_serial("CHAT_OK", 5))
    check_line(b, "m25_b_recv", 0, line, CHAT_TEXT,
               f"B renders '{USER_A}:'-prefixed message (exact font pixels)")
    check_line(a, "m25_a_sent", 0, line, CHAT_MINE,
               "A renders its own message (mine color)")

    a.quit()
    b.quit()
    finish()


if __name__ == "__main__":
    main()
