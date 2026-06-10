#!/usr/bin/env python3
"""Tiny QMP client: connect to QEMU's QMP unix socket, take a screendump
(PPM for pixel checks + PNG for humans), then quit QEMU.

Usage: qmp.py <socket> <out.ppm> <out.png>
"""
import json
import socket
import sys
import time


def main():
    sock_path, ppm, png = sys.argv[1], sys.argv[2], sys.argv[3]
    s = socket.socket(socket.AF_UNIX)
    s.connect(sock_path)
    f = s.makefile("rw")

    def recv():
        while True:
            msg = json.loads(f.readline())
            if "event" not in msg:
                return msg

    def cmd(name, **args):
        f.write(json.dumps({"execute": name, "arguments": args}) + "\n")
        f.flush()
        resp = recv()
        if "error" in resp:
            raise RuntimeError(f"{name}: {resp['error']}")
        return resp

    recv()  # greeting
    cmd("qmp_capabilities")
    cmd("screendump", filename=ppm, format="ppm")
    cmd("screendump", filename=png, format="png")
    cmd("quit")
    time.sleep(0.2)
    print("screendump written:", ppm, png)


if __name__ == "__main__":
    main()
