"""Shared QMP-driving + screendump-checking helpers for the GUI proofs."""
import json
import socket
import sys
import time

W, H = 1024, 768
ABS_MAX = 32767

failures = []


class Qmp:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(path)
        self.f = self.sock.makefile("rw")
        self._recv()  # greeting
        self.cmd("qmp_capabilities")

    def _recv(self):
        while True:
            msg = json.loads(self.f.readline())
            if "event" not in msg:
                return msg

    def cmd(self, name, **args):
        self.f.write(json.dumps({"execute": name, "arguments": args}) + "\n")
        self.f.flush()
        resp = self._recv()
        if "error" in resp:
            raise RuntimeError(f"{name}: {resp['error']}")
        return resp


class Driver:
    def __init__(self, qmp_path, serial_path, shots):
        self.q = Qmp(qmp_path)
        self.serial_path = serial_path
        self.shots = shots

    def send(self, events):
        self.q.cmd("input-send-event", events=events)
        time.sleep(0.06)

    def move(self, x, y):
        self.send([
            {"type": "abs", "data": {"axis": "x", "value": round(x * ABS_MAX / (W - 1))}},
            {"type": "abs", "data": {"axis": "y", "value": round(y * ABS_MAX / (H - 1))}},
        ])

    def button(self, down):
        self.send([{"type": "btn", "data": {"down": down, "button": "left"}}])

    def click(self, x, y):
        self.move(x, y)
        self.button(True)
        self.button(False)

    def drag(self, x0, y0, x1, y1, steps=8):
        self.move(x0, y0)
        self.button(True)
        for i in range(1, steps + 1):
            self.move(x0 + (x1 - x0) * i // steps, y0 + (y1 - y0) * i // steps)
        self.button(False)

    def type_text(self, text):
        for ch in text:
            shift = ch.isupper()
            qcode = {" ": "spc", "\n": "ret", ".": "dot", "-": "minus",
                     "/": "slash"}.get(ch.lower(), ch.lower())
            if shift:
                self.send([{"type": "key", "data": {
                    "down": True, "key": {"type": "qcode", "data": "shift"}}}])
            for down in (True, False):
                self.send([{"type": "key", "data": {
                    "down": down, "key": {"type": "qcode", "data": qcode}}}])
            if shift:
                self.send([{"type": "key", "data": {
                    "down": False, "key": {"type": "qcode", "data": "shift"}}}])

    def dump(self, name):
        time.sleep(0.4)
        ppm = f"{self.shots}/{name}.ppm"
        self.q.cmd("screendump", filename=ppm, format="ppm")
        self.q.cmd("screendump", filename=f"{self.shots}/{name}.png", format="png")
        time.sleep(0.1)
        return Image(ppm)

    def serial(self):
        return open(self.serial_path, errors="replace").read()

    def wait_serial(self, needle, timeout=20.0, after=0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if needle in self.serial()[after:]:
                return True
            time.sleep(0.1)
        return False

    def quit(self):
        try:
            self.q.cmd("quit")
        except Exception:
            pass


class Image:
    def __init__(self, path):
        data = open(path, "rb").read()
        parts = data.split(b"\n", 3)
        assert parts[0] == b"P6" and parts[2] == b"255"
        self.w, self.h = map(int, parts[1].split())
        self.px = parts[3]

    def at(self, x, y):
        i = (y * self.w + x) * 3
        return (self.px[i], self.px[i + 1], self.px[i + 2])


def check(label, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'} {label}{': ' + detail if detail else ''}")
    if not ok:
        failures.append(label)


def check_px(img, label, x, y, want, invert=False):
    got = img.at(x, y)
    ok = (got != want) if invert else (got == want)
    rel = "!=" if invert else "=="
    check(label, ok, f"@({x},{y}) got rgb{got}, want {rel} rgb{want}")


def finish():
    print(f"\n{'FAILED: ' + str(len(failures)) + ' checks' if failures else 'ALL CHECKS PASSED'}")
    sys.exit(1 if failures else 0)
