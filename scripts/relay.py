#!/usr/bin/env python3
"""Veil OS chat relay (M26): a tiny TCP fan-out server.

Protocol (newline-framed text; MSG carries a length-prefixed body):

  client -> server on connect:  HELLO <username>\\n
  server -> all others:         JOIN <username>\\n      (and the existing
                                 roster is replayed to the newcomer)
  server -> all:                PART <username>\\n      (on disconnect)
  client -> server:             MSG <from> <to|*> <len>\\n<body bytes>
  server -> recipients:         MSG <from> <to|*> <len>\\n<body bytes>

Public messages (to == '*') fan out to everyone including the sender;
direct messages go only to the named recipient, echoed back to the sender.

Self-hosted on the demo Mac mini; QEMU guests reach it via the slirp host
gateway at 10.0.2.2:7778. Runs as LaunchAgent com.veil.relay. Cloudflare
ingress relay.henryratterman.com is optional (only needed for off-box
clients). Stdlib only.

Usage: relay.py [port]   (default 7778)
"""
import socket
import sys
import threading

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 7778


class Conn:
    def __init__(self, sock, addr):
        self.sock = sock
        self.addr = addr
        self.name = None
        self.buf = b""
        self.lock = threading.Lock()  # serialize writes to this socket

    def fill(self):
        chunk = self.sock.recv(4096)
        if not chunk:
            raise ConnectionError
        self.buf += chunk

    def read_line(self):
        while b"\n" not in self.buf:
            self.fill()
        line, self.buf = self.buf.split(b"\n", 1)
        return line.decode("utf-8", "replace")

    def read_exact(self, n):
        while len(self.buf) < n:
            self.fill()
        data, self.buf = self.buf[:n], self.buf[n:]
        return data

    def send(self, data):
        with self.lock:
            try:
                self.sock.sendall(data)
            except OSError:
                pass


class Relay:
    def __init__(self):
        self.conns = {}          # name -> Conn
        self.lock = threading.Lock()

    def broadcast(self, data, exclude=None):
        with self.lock:
            targets = [c for n, c in self.conns.items() if n != exclude]
        for c in targets:
            c.send(data)

    def join(self, conn):
        with self.lock:
            roster = list(self.conns.keys())
            self.conns[conn.name] = conn
        # Replay the existing roster to the newcomer...
        for n in roster:
            conn.send(f"JOIN {n}\n".encode())
        # ...and announce the newcomer to everyone else.
        self.broadcast(f"JOIN {conn.name}\n".encode(), exclude=conn.name)
        print(f"+ {conn.name} ({conn.addr[1]}) — {len(roster) + 1} online", flush=True)

    def part(self, conn):
        if conn.name is None:
            return
        with self.lock:
            if self.conns.get(conn.name) is conn:
                del self.conns[conn.name]
        self.broadcast(f"PART {conn.name}\n".encode())
        print(f"- {conn.name}", flush=True)

    def route(self, frm, to, body):
        frame = f"MSG {frm} {to} {len(body)}\n".encode() + body
        if to == "*":
            self.broadcast(frame)
        else:
            with self.lock:
                dst = self.conns.get(to)
                src = self.conns.get(frm)
            if dst:
                dst.send(frame)
            if src and src is not dst:
                src.send(frame)  # echo DM back to sender

    def serve_client(self, sock, addr):
        conn = Conn(sock, addr)
        try:
            hello = conn.read_line().split()
            if len(hello) < 2 or hello[0] != "HELLO":
                return
            conn.name = hello[1][:20]
            # Disambiguate a duplicate name so routing stays unique.
            with self.lock:
                base, i = conn.name, 1
                while conn.name in self.conns:
                    conn.name = f"{base}{i}"
                    i += 1
            self.join(conn)
            while True:
                line = conn.read_line().split(" ")
                if line[0] == "MSG" and len(line) >= 4:
                    frm, to, ln = line[1], line[2], int(line[3])
                    body = conn.read_exact(ln)
                    self.route(frm, to, body)
        except (ConnectionError, OSError, ValueError):
            pass
        finally:
            self.part(conn)
            try:
                sock.close()
            except OSError:
                pass


def main():
    relay = Relay()
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    s.bind(("0.0.0.0", PORT))
    s.listen(64)
    print(f"veil relay on :{PORT}", flush=True)
    while True:
        sock, addr = s.accept()
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        threading.Thread(target=relay.serve_client, args=(sock, addr),
                         daemon=True).start()


if __name__ == "__main__":
    main()
