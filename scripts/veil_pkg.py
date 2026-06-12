#!/usr/bin/env python3
"""Veil package tool + registry server.

Build a .veil package (a ZIP of manifest.toml + main.wasm + assets):
    python3 scripts/veil_pkg.py pack --manifest app/manifest.toml \\
        --wasm app/main.wasm --asset app/icon.png --out hello.veil

Serve a registry (the in-OS `pkg install <name>` fetches <base>/<name>.veil):
    python3 scripts/veil_pkg.py serve --root ./registry --port 8080

A manifest.toml looks like:
    name = "hello"
    version = "1.0.0"
    description = "A demo app"
    author = "henry"
    entry = "main.wasm"
    permissions = ["storage", "net"]
"""
import argparse
import http.server
import os
import zipfile


def pack(args):
    if not os.path.isfile(args.manifest):
        raise SystemExit(f"no manifest: {args.manifest}")
    # ZIP_STORED keeps the archive trivially parseable in-OS; ZIP_DEFLATED also
    # works (the OS uses its png::inflate). Default to STORED for small apps.
    comp = zipfile.ZIP_DEFLATED if args.deflate else zipfile.ZIP_STORED
    with zipfile.ZipFile(args.out, "w", comp) as z:
        z.write(args.manifest, "manifest.toml")
        if args.wasm:
            z.write(args.wasm, "main.wasm")
        for a in args.asset or []:
            z.write(a, os.path.basename(a))
    print(f"wrote {args.out} ({os.path.getsize(args.out)} bytes)")


def serve(args):
    os.chdir(args.root)
    handler = http.server.SimpleHTTPRequestHandler
    httpd = http.server.HTTPServer((args.host, args.port), handler)
    print(f"Veil registry serving {os.path.abspath(args.root)} on {args.host}:{args.port}")
    print(f"  in Veil:  pkg install <name>   (fetches /<name>.veil)")
    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("pack", help="build a .veil package")
    p.add_argument("--manifest", required=True)
    p.add_argument("--wasm")
    p.add_argument("--asset", action="append")
    p.add_argument("--out", required=True)
    p.add_argument("--deflate", action="store_true", help="compress entries")
    p.set_defaults(func=pack)

    s = sub.add_parser("serve", help="serve a package registry over HTTP")
    s.add_argument("--root", default="./registry")
    s.add_argument("--port", type=int, default=8080)
    s.add_argument("--host", default="0.0.0.0")
    s.set_defaults(func=serve)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
