#!/usr/bin/env python3
"""M24 GUI proof: the audio app plays the test tone through virtio-sound.

Launch Audio from the taskbar, click Play, confirm the stream starts and
the window shows "playing", then require a clean AUDIO_OK. (Audibility is
the subjective demo.sh check; here we verify the driver + UI end to end.)

Audio window (wm.rs): (360, 300), content 300x130. Without a NIC the Chat
launcher is hidden, so Audio is taskbar idx 6: x = 70 + 6*78 + 36 = 574.
Play button center ~ (512, 427).
"""
import sys

from guilib import Driver, check, finish

AUDIO_BTN = (574, 768 - 20)
PLAY = (512, 427)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("SND_OK on serial", "SND_OK" in d.serial())

    mark = len(d.serial())
    d.click(*AUDIO_BTN)
    check("audio window open", d.wait_serial("AUDIO: window open", 5, mark))

    mark = len(d.serial())
    d.click(*PLAY)
    check("play started", d.wait_serial("AUDIO: play TONE.WAV", 5, mark))
    check("stream started on the device", d.wait_serial("SND: stream started", 5, mark))

    d.move(1000, 700)
    d.dump("m24_playing")  # window shows "playing" + elapsed

    check("tone played to completion (AUDIO_OK)", d.wait_serial("AUDIO_OK", 15, mark))
    d.quit()
    finish()


if __name__ == "__main__":
    main()
