"""M26 proof: three Veil instances on one chat relay (scripts/relay.py).

  - ann sends a PUBLIC message -> renders in ann (mine), bob and cid.
  - ann sends a DM to bob       -> renders in ann (echo) and bob, NOT cid.
  - the online-user panel in every window lists all three usernames.

All assertions are exact font-pixel comparisons rendered host-side from
src/font.rs, over the chat log (left) and the user panel (right), plus
serial checks for relay routing + the DM_OK sentinel.

Chat geometry (wm.rs): window (40,380), content 440x300. Log row i at
canvas (6, 4+16*i); user panel at canvas x = 440-80 = 360, names at
(360+16, 24+16*i) sorted alphabetically.
"""
import re
import sys

from guilib import Driver, check, finish

WIN_X, WIN_Y = 40, 380
BORDER, TITLE_H = 2, 22
CW = 440
PANEL_W = 80
LOG_X = WIN_X + BORDER + 6                  # 48
LOG_Y = WIN_Y + BORDER + TITLE_H + 4        # 408
PANEL_NAME_X = WIN_X + BORDER + (CW - PANEL_W) + 16   # 418
PANEL_NAME_Y0 = WIN_Y + BORDER + TITLE_H + 24         # 428

CHAT_BG = (0xF8, 0xF6, 0xF0)
CHAT_TEXT = (0x20, 0x28, 0x30)
CHAT_MINE = (0x20, 0x60, 0xA0)
CHAT_DM = (0xB0, 0x58, 0x38)
PANEL_TEXT = (0x30, 0x38, 0x40)
PANEL_BG = (0xE8, 0xE6, 0xE0)

CHAT_BTN = (496, 768 - 20)
PUBLIC = "hello everyone"
DM = "secret for bob"


def load_font():
    glyphs = {}
    src = open("src/font.rs").read()
    for i, m in enumerate(re.finditer(r"\[((?:0x[0-9a-f]{2},?\s*){16})\]", src)):
        rows = [int(b, 16) for b in re.findall(r"0x[0-9a-f]{2}", m.group(1))]
        glyphs[chr(0x20 + i)] = rows
    assert len(glyphs) == 95, f"parsed {len(glyphs)} glyphs"
    return glyphs


GLYPHS = load_font()


def render_expected(text, fg, bg):
    out = bytearray()
    for row in range(16):
        for ch in text:
            bits = GLYPHS.get(ch, GLYPHS[" "])[row]
            for n in range(8):
                out += bytes(fg if bits >> n & 1 else bg)
    return bytes(out)


def strip(img, x, y, width_px):
    out = bytearray()
    for yy in range(y, y + 16):
        i = (yy * img.w + x) * 3
        out += img.px[i:i + width_px * 3]
    return bytes(out)


def check_text(d, shot, x, y, text, fg, bg, label):
    d.move(1000, 740)
    img = d.dump(shot)
    got = strip(img, x, y, len(text) * 8)
    want = render_expected(text, fg, bg)
    diff = sum(a != b for a, b in zip(got, want))
    check(label, got == want, "" if got == want else f"{diff} bytes differ")


def check_blank(d, shot, x, y, width_px, label):
    d.move(1000, 740)
    img = d.dump(shot)
    got = strip(img, x, y, width_px)
    want = bytes(CHAT_BG) * (width_px * 16)
    check(label, got == want, "" if got == want else "pixels present (DM leaked?)")


def open_chat(d, who):
    d.click(*CHAT_BTN)
    check(f"{who} chat opens (relay)", d.wait_serial("relay", 25))


def main():
    qa, sa, qb, sb, qc, sc, shots = sys.argv[1:8]
    a = Driver(qa, sa, shots)
    b = Driver(qb, sb, shots)
    c = Driver(qc, sc, shots)

    for d, who in ((a, "ann"), (b, "bob"), (c, "cid")):
        check(f"{who} desktop up", d.wait_serial("WM_OK", 60))

    # Open chat sequentially so the JOIN ordering is deterministic.
    open_chat(a, "ann")
    open_chat(b, "bob")
    open_chat(c, "cid")

    # Every instance must end up seeing all three users online.
    for d, who in ((a, "ann"), (b, "bob"), (c, "cid")):
        check(f"{who} roster reaches 3", d.wait_serial("(users 3)", 20))

    # The panel renders the usernames (verify in cid, which has the full set).
    for i, name in enumerate(("ann", "bob", "cid")):
        check_text(c, f"m26_panel_{name}", PANEL_NAME_X, PANEL_NAME_Y0 + 16 * i,
                   name, PANEL_TEXT, PANEL_BG, f"cid panel lists '{name}'")

    # --- public message from ann -------------------------------------
    ma, mb, mc = len(a.serial()), len(b.serial()), len(c.serial())
    a.click(240, 500)          # focus ann chat log
    a.type_text(PUBLIC + "\n")
    pub = f"ann: {PUBLIC}"
    check("ann sent public", a.wait_serial("CHAT: sent MSG to '*'", 10, ma))
    check("bob got public", b.wait_serial(f'CHAT: rx "{pub}"', 15, mb))
    check("cid got public", c.wait_serial(f'CHAT: rx "{pub}"', 15, mc))
    check_text(b, "m26_b_pub", LOG_X, LOG_Y, pub, CHAT_TEXT, CHAT_BG,
               "bob renders public message")
    check_text(c, "m26_c_pub", LOG_X, LOG_Y, pub, CHAT_TEXT, CHAT_BG,
               "cid renders public message")
    check_text(a, "m26_a_pub", LOG_X, LOG_Y, pub, CHAT_MINE, CHAT_BG,
               "ann renders its own public message (mine colour)")

    # --- DM from ann to bob ------------------------------------------
    ma, mb, mc = len(a.serial()), len(b.serial()), len(c.serial())
    a.click(430, 450)          # click 'bob' in ann's user panel -> DM mode
    check("ann selects DM target bob", a.wait_serial("dm target -> bob", 8, ma))
    a.type_text(DM + "\n")
    bob_sees = f"ann -> you: {DM}"
    ann_sees = f"ann -> bob: {DM}"
    check("bob got DM", b.wait_serial(f'CHAT: rx "{bob_sees}"', 15, mb))
    check("bob emits DM_OK", b.wait_serial("DM_OK", 5))
    check("ann emits DM_OK", a.wait_serial("DM_OK", 5))
    check_text(b, "m26_b_dm", LOG_X, LOG_Y + 16, bob_sees, CHAT_DM, CHAT_BG,
               "bob renders the DM (terracotta)")
    check_text(a, "m26_a_dm", LOG_X, LOG_Y + 16, ann_sees, CHAT_DM, CHAT_BG,
               "ann renders the DM echo (terracotta)")
    # cid must NOT have received the DM: log row 1 stays background.
    import time
    time.sleep(1.0)
    check("cid never saw the DM",
          'secret for bob' not in c.serial()[mc:])
    check_blank(c, "m26_c_nodm", LOG_X, LOG_Y + 16, len(bob_sees) * 8,
                "cid DM row is blank (DM not delivered)")

    a.quit()
    b.quit()
    c.quit()
    finish()


if __name__ == "__main__":
    main()
