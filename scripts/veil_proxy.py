#!/usr/bin/env python3
"""M32-A4: a tiny HTTP proxy that gives the Veil browser the real internet.

The guest can only speak plain HTTP and renders a small HTML/CSS subset, so it
sends every external request here (reachable at 10.0.2.2:7779 through QEMU's
slirp gateway) as an absolute-form request:

    GET http://example.com/path HTTP/1.1
    GET https://lite.cnn.com/    HTTP/1.1

We fetch the real URL on the host (HTTPS and redirects included), strip it down
to what the Veil browser can render, rewrite links to absolute URLs so they
keep routing back through the proxy, and return clean HTTP/1.1.
"""
import gzip
import http.server
import re
import socketserver
import sys
import urllib.parse
import urllib.request

PORT = 7779
UA = "Mozilla/5.0 (VeilOS; from-scratch browser) Gecko"
# Tags whose entire subtree is useless (or harmful) to a JS-less text browser.
DROP_TREES = (
    "script", "style", "svg", "noscript", "iframe", "form", "button",
    "select", "input", "textarea", "canvas", "video", "audio", "object",
    "template", "head",
)


def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "text/html"})
    with urllib.request.urlopen(req, timeout=15) as r:
        data = r.read(4_000_000)
        ctype = r.headers.get_content_type()
        enc = (r.headers.get("Content-Encoding") or "").lower()
        final = r.geturl()
    if "gzip" in enc:
        data = gzip.decompress(data)
    return data, ctype, final


def strip_html(html, base):
    html = re.sub(r"<!--.*?-->", "", html, flags=re.S)
    for tag in DROP_TREES:
        html = re.sub(rf"<{tag}\b.*?</{tag}>", " ", html, flags=re.S | re.I)
        html = re.sub(rf"<{tag}\b[^>]*/?>", " ", html, flags=re.I)
    html = re.sub(r"<img\b[^>]*>", " [image] ", html, flags=re.I)
    # Absolutise links so the guest's clicks come back through the proxy.
    html = re.sub(
        r'href\s*=\s*"([^"]*)"',
        lambda m: 'href="%s"' % urllib.parse.urljoin(base, m.group(1)),
        html,
        flags=re.I,
    )
    html = re.sub(
        r"href\s*=\s*'([^']*)'",
        lambda m: 'href="%s"' % urllib.parse.urljoin(base, m.group(1)),
        html,
        flags=re.I,
    )
    # Drop inline event handlers and style attributes the browser ignores anyway.
    html = re.sub(r'\son\w+\s*=\s*"[^"]*"', "", html, flags=re.I)
    html = re.sub(r"\sstyle\s*=\s*\"[^\"]*\"", "", html, flags=re.I)
    return html


def page(title, body):
    return ("<html><body><h1>%s</h1>%s</body></html>" % (title, body)).encode()


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        url = self.path
        if not (url.startswith("http://") or url.startswith("https://")):
            return self._send(page("veil proxy", "<p>expected an absolute URL</p>"))
        try:
            data, ctype, final = fetch(url)
        except Exception as e:  # noqa: BLE001 — surface any failure to the guest
            return self._send(page("proxy error",
                                    "<p>%s: %s</p><p>%s</p>" % (type(e).__name__, e, url)))
        if "html" not in ctype:
            return self._send(page(ctype or "unknown",
                                   "<p>not HTML (%d bytes)</p>" % len(data)))
        try:
            html = data.decode("utf-8", "replace")
        except Exception:  # noqa: BLE001
            html = data.decode("latin-1", "replace")
        body = strip_html(html, final).encode("utf-8", "replace")
        sys.stdout.write("proxy: %s -> %d bytes\n" % (url, len(body)))
        sys.stdout.flush()
        self._send(body)

    def _send(self, body):
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        try:
            self.wfile.write(body)
        except BrokenPipeError:
            pass

    def log_message(self, *a):
        pass


class Server(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


if __name__ == "__main__":
    with Server(("127.0.0.1", PORT), Handler) as s:
        sys.stdout.write("veil proxy listening on 127.0.0.1:%d\n" % PORT)
        sys.stdout.flush()
        s.serve_forever()
