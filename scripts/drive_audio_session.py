#!/usr/bin/env python3
"""Freeze-fix proof: in a hosted-style session (wav FIFO audiodev), complete
first-boot setup, open Audio, press Play, and require the stream to finish
(AUDIO_OK) AND the desktop to stay responsive afterwards (framebuffer still
changes when the cursor moves). Before the fix, Play froze the whole VM."""
import sys
import time

from guilib import Driver, check, finish


def key(d, qc):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qc}}}])


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])

    check("setup screen shown", d.wait_serial("SETUP: first boot", 60))
    d.type_text("demo")
    key(d, "ret")
    check("SETUP_OK", d.wait_serial("SETUP_OK", 10))
    check("desktop reached", d.wait_serial("WM_OK", 20))

    # Audio launcher: with a NIC present the order is
    # edit,clock,browser,paint,shell,chat,viewer,audio,files -> idx 7.
    mark = len(d.serial())
    d.click(70 + 7 * 78 + 36, 768 - 20)
    check("audio window open", d.wait_serial("AUDIO: window open", 5, mark))

    d.move(1000, 700)
    before = d.dump("audiofix_before")

    mark = len(d.serial())
    d.click(512, 427)  # Play button
    check("play requested", d.wait_serial("AUDIO: play TONE.WAV", 5, mark))
    check("stream started", d.wait_serial("SND: stream started", 5, mark))
    # The whole point: if QEMU froze on the FIFO write, AUDIO_OK never comes.
    check("NO FREEZE — stream completed (AUDIO_OK)", d.wait_serial("AUDIO_OK", 25, mark))

    # And the VM is still alive: moving the cursor changes the framebuffer.
    d.move(150, 150)
    time.sleep(0.3)
    d.move(600, 450)
    after = d.dump("audiofix_after")
    check("desktop responsive after playback (framebuffer changed)",
          before.px != after.px)

    d.quit()
    finish()


if __name__ == "__main__":
    main()
