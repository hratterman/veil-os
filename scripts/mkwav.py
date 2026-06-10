#!/usr/bin/env python3
"""Generate a 440 Hz sine test tone as a 16-bit stereo 44.1 kHz WAV (the
M24 audio sample). Pure stdlib. Usage: mkwav.py <out.wav> [seconds]"""
import math
import struct
import sys

RATE = 44100
CH = 2
FREQ = 440.0


def main():
    out = sys.argv[1]
    secs = float(sys.argv[2]) if len(sys.argv) > 2 else 3.0
    n = int(RATE * secs)
    frames = bytearray()
    amp = 9000  # comfortable level, well below full scale
    for i in range(n):
        # Short fade in/out to avoid clicks.
        env = min(1.0, i / 1000.0, (n - i) / 1000.0)
        s = int(amp * env * math.sin(2 * math.pi * FREQ * i / RATE))
        frames += struct.pack("<hh", s, s)  # stereo: same on both channels
    data = bytes(frames)
    byte_rate = RATE * CH * 2
    with open(out, "wb") as f:
        f.write(b"RIFF")
        f.write(struct.pack("<I", 36 + len(data)))
        f.write(b"WAVE")
        f.write(b"fmt ")
        f.write(struct.pack("<IHHIIHH", 16, 1, CH, RATE, byte_rate, CH * 2, 16))
        f.write(b"data")
        f.write(struct.pack("<I", len(data)))
        f.write(data)
    print(f"{out}: {secs}s {FREQ:.0f}Hz {RATE}Hz {CH}ch 16-bit, {len(data)} PCM bytes")


if __name__ == "__main__":
    main()
