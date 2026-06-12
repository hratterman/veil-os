"""M18 proof, one phase per boot (scripts/m18_test.sh runs both):

  save: fresh disk -> editor created NOTE.TXT; type a known string, snapshot
        the text-area pixels, click SAV, require EDITOR_OK on serial.
  load: same disk, new boot -> click LOD, require EDITOR_OK, snapshot the
        same region and require it pixel-identical to the save-phase
        snapshot (and not blank) — the typed text survived the reboot.

Editor geometry (desktop.rs / wm.rs, mirrored here): window at (40, 40),
content 420x300, BORDER 2, TITLE_H 22, TOOLBAR_H 30. SAV button at
canvas x cw-52..cw-8, LOD at cw-100..cw-56, both y 2..26.
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

WIN_X, WIN_Y, CW, CH = 40, 40, 420, 300
CONTENT_X = WIN_X + 2
CONTENT_Y = WIN_Y + 2 + 22
TOOLBAR_H = 30
SAV = (CONTENT_X + CW - 52 + 22, CONTENT_Y + 14)
LOD = (CONTENT_X + CW - 100 + 22, CONTENT_Y + 14)
# Text area in screen coords (toolbar excluded).
REGION = (CONTENT_X, CONTENT_Y + TOOLBAR_H, CW, CH - TOOLBAR_H)

TYPED = "Hello from Veil M18\nthis line survived a reboot"


def region_bytes(img):
    x0, y0, w, h = REGION
    rows = []
    for y in range(y0, y0 + h):
        i = (y * img.w + x0) * 3
        rows.append(img.px[i:i + w * 3])
    return b"".join(rows)


def main():
    qmp, serial_path, shots, phase = sys.argv[1:5]
    d = Driver(qmp, serial_path, shots)
    snap = f"{shots}/m18_region.bin"

    # UX overhaul: nothing opens at boot. Launch the Editor from the
    # taskbar (idx 0: x=70+0*78+36=106, bottom 40px strip).
    mark0 = len(d.serial())
    d.click(*taskbar_xy(d, "edit"))
    check("editor launched", d.wait_serial("WM: launch 'edit'", 5, mark0))

    if phase == "save":
        check("editor window opened", d.wait_serial("EDITOR: window open on NOTE.TXT", 10))
        check("NOTE.TXT created on fresh disk", "EDITOR: NOTE.TXT missing -> new file" in d.serial())
        d.click(200, 200)  # focus the editor (clicks in the text area are inert)
        for line in TYPED.split("\n"):
            d.type_text(line + "\n")
        # Move the cursor to the start so the snapshot's cursor block matches the
        # load phase (which opens with the cursor at offset 0).
        for q in ("up", "up", "home"):
            d.send([{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": q}}}])
            d.send([{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": q}}}])
        # drop the trailing enter we just typed to match TYPED exactly:
        # easier to just keep it — the buffer ends with '\n', deterministic
        # in both phases since LOD reloads the same bytes.
        d.move(1000, 700)  # park the mouse out of the snapshot region
        img = d.dump("m18_typed")
        region = region_bytes(img)
        check("typed text is on screen", region != b"\xff" * len(region))
        mark = len(d.serial())
        d.click(*SAV)
        check("SAV wrote NOTE.TXT",
              d.wait_serial("EDITOR: saved 48 bytes to NOTE.TXT", 10, mark))
        check("EDITOR_OK emitted", d.wait_serial("EDITOR_OK", 5, mark))
        open(snap, "wb").write(region)
        d.quit()
    elif phase == "load":
        check("editor reopened NOTE.TXT from disk",
              d.wait_serial("EDITOR: opened NOTE.TXT (48 bytes)", 10))
        mark = len(d.serial())
        d.click(*LOD)
        check("LOD re-read NOTE.TXT",
              d.wait_serial("EDITOR: loaded 48 bytes from NOTE.TXT", 10, mark))
        check("EDITOR_OK emitted", d.wait_serial("EDITOR_OK", 5, mark))
        d.move(1000, 700)
        img = d.dump("m18_reloaded")
        region = region_bytes(img)
        saved = open(snap, "rb").read()
        check("text area not blank", region != b"\xff" * len(region))
        check("text area pixel-identical across reboot", region == saved,
              f"{sum(a != b for a, b in zip(region, saved))} differing bytes"
              if region != saved else "")
        d.quit()
    else:
        check(f"unknown phase {phase}", False)
    finish()


if __name__ == "__main__":
    main()
