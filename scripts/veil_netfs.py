#!/usr/bin/env python3
"""VeilNetFS host server — serve a directory tree to Veil over the netfs protocol.

Run on the Detroit Mac mini (the "host"):
    python3 scripts/veil_netfs.py --root ~/shared --port 2049

Then inside a Veil session:
    mount veil-host:/ /mnt/host
    ls /mnt/host
    cat /mnt/host/somefile.txt

Protocol (one request line per connection; the client reads until EOF):
    LIST <path>\\n   -> "OK <n>\\n" then n lines "<D|F> <size> <name>"
    READ <path>\\n   -> "OK <len>\\n" then <len> raw bytes
    STAT <path>\\n   -> "OK <D|F> <size>\\n"
    (errors)        -> "ERR <message>\\n"

Read-only. Paths are confined to --root (no escaping via .. or symlinks).
"""
import argparse
import os
import socketserver


def safe_join(root, path):
    # Confine the requested path to root; reject traversal.
    p = path.lstrip("/")
    full = os.path.realpath(os.path.join(root, p))
    root_real = os.path.realpath(root)
    if full != root_real and not full.startswith(root_real + os.sep):
        return None
    return full


class Handler(socketserver.StreamRequestHandler):
    def handle(self):
        root = self.server.root
        line = self.rfile.readline().decode("utf-8", "replace").strip()
        parts = line.split(" ", 1)
        cmd = parts[0] if parts else ""
        path = parts[1] if len(parts) > 1 else "/"
        full = safe_join(root, path)
        if full is None:
            self.wfile.write(b"ERR forbidden\n")
            return
        try:
            if cmd == "LIST":
                if not os.path.isdir(full):
                    self.wfile.write(b"ERR no such directory\n")
                    return
                entries = sorted(os.listdir(full))
                self.wfile.write(f"OK {len(entries)}\n".encode())
                for name in entries:
                    fp = os.path.join(full, name)
                    is_dir = os.path.isdir(fp)
                    size = 0 if is_dir else os.path.getsize(fp)
                    self.wfile.write(f"{'D' if is_dir else 'F'} {size} {name}\n".encode())
            elif cmd == "READ":
                if not os.path.isfile(full):
                    self.wfile.write(b"ERR no such file\n")
                    return
                with open(full, "rb") as f:
                    data = f.read()
                self.wfile.write(f"OK {len(data)}\n".encode())
                self.wfile.write(data)
            elif cmd == "STAT":
                if not os.path.exists(full):
                    self.wfile.write(b"ERR no such path\n")
                    return
                is_dir = os.path.isdir(full)
                size = 0 if is_dir else os.path.getsize(full)
                self.wfile.write(f"OK {'D' if is_dir else 'F'} {size}\n".encode())
            else:
                self.wfile.write(b"ERR bad command\n")
        except OSError as e:
            self.wfile.write(f"ERR {e}\n".encode())


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=os.getcwd(), help="directory to serve")
    ap.add_argument("--port", type=int, default=2049)
    ap.add_argument("--host", default="0.0.0.0")
    args = ap.parse_args()
    srv = Server((args.host, args.port), Handler)
    srv.root = os.path.realpath(args.root)
    print(f"VeilNetFS serving {srv.root} on {args.host}:{args.port}")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
