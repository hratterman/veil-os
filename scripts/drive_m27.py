"""M27 proof driver. Two phases over two sequential boots of one disk:

  phase 'setup'  (first boot, no USER.TXT): drive the setup screen — type a
                 name, arrow the timezone to UTC-5, Enter; verify SETUP_OK
                 and that the desktop replaces the setup card (pixel check).
  phase 'verify' (second boot, USER.TXT now present): assert the setup
                 screen does NOT appear, the username persisted (Chat opens
                 as 'testuser'), and the timezone took (TZ -18000s).
"""
import sys
import time

from guilib import Driver, check, finish

NAME = "testuser"
CARD = (0x15, 0x1D, 0x2A)        # setup card fill
DESKTOP_BG = (0x28, 0x48, 0x58)  # bare desktop


def key(d, qcode):
    for down in (True, False):
        d.send([{"type": "key", "data": {
            "down": down, "key": {"type": "qcode", "data": qcode}}}])


def phase_setup(d):
    check("setup screen shown", d.wait_serial("SETUP: first boot", 60))
    img = d.dump("m27_setup")
    check("setup card visible", img.at(512, 400) == CARD,
          f"center px {img.at(512, 400)} want {CARD}")
    d.type_text(NAME)
    for _ in range(10):       # UTC+0 -> UTC-5 (10 x 30 min)
        key(d, "left")
    key(d, "ret")
    check("SETUP_OK emitted", d.wait_serial("SETUP_OK", 10))
    check("name+tz logged", d.wait_serial("name='testuser' tz=-5", 5))
    check("desktop reached", d.wait_serial("WM_OK", 20))
    time.sleep(0.5)
    img = d.dump("m27_desktop")
    check("desktop replaced setup card", img.at(512, 400) == DESKTOP_BG,
          f"center px {img.at(512, 400)} want {DESKTOP_BG}")
    d.quit()


def phase_verify(d):
    check("desktop reached (no setup)", d.wait_serial("WM_OK", 60))
    check("setup screen NOT shown again", "SETUP: first boot" not in d.serial())
    check("timezone persisted (-18000s)", d.wait_serial("UTC offset -18000s", 5))
    d.click(496, 768 - 20)  # open Chat from taskbar
    check("username persisted", d.wait_serial("CHAT: window open as 'testuser'", 20))
    d.quit()


def main():
    phase, qmp, ser, shots = sys.argv[1:5]
    d = Driver(qmp, ser, shots)
    if phase == "setup":
        phase_setup(d)
    else:
        phase_verify(d)
    finish()


if __name__ == "__main__":
    main()
