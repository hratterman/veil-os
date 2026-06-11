"""M28 driver: open the Audio app and click Play so the kernel streams
TONE.WAV through virtio-sound -> QEMU's wav FIFO audiodev. The PCM tap +
WebSocket forwarding are verified out-of-band by scripts/ws_probe.js; this
driver just triggers playback (the spec's "send a play action").

Layout matches drive_m24 (no NIC -> Audio is taskbar idx 6, x=574; the
audio window opens at (360,300), Play button center ~ (512,427)).
"""
import sys

from guilib import Driver, check, finish

AUDIO_BTN = (574, 768 - 20)
PLAY = (512, 427)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("virtio-sound ready", d.wait_serial("SND_OK", 60))
    check("desktop up", d.wait_serial("WM_OK", 60))

    mark = len(d.serial())
    d.click(*AUDIO_BTN)
    check("audio window open", d.wait_serial("AUDIO: window open", 5, mark))

    mark = len(d.serial())
    d.click(*PLAY)
    check("play started", d.wait_serial("AUDIO: play TONE.WAV", 5, mark))
    check("stream started on the device", d.wait_serial("SND: stream started", 5, mark))
    # Let the tone play out so the FIFO -> WS probe has time to collect PCM.
    check("tone played to completion", d.wait_serial("AUDIO_OK", 15, mark))
    d.quit()
    finish()


if __name__ == "__main__":
    main()
