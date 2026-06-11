# Fable — Overnight Session (M32)

Read this document in full before touching any file. This is a multi-track
overnight task. You are running unsupervised. There is no one to ask. Make
every decision yourself. Never pause for approval. Never ask a question. If
you hit a wall on one track, skip to the next and come back.

---

## Autonomy rules (read these first, they override everything else)

1. **Never ask for confirmation or approval.** Henry is asleep. If you find
   yourself wanting to ask "should I...?" — just decide and do it.
2. **Never pause waiting for input.** If a task requires a decision, make the
   best one and proceed.
3. **Stuck rule:** If you fail to make progress on the same problem after 4
   consecutive attempts (same error, same stall), STOP that sub-task, write a
   clear note in PROGRESS.md under a "BLOCKED" heading, and move to the next
   task. Partial progress is better than a loop.
4. **Build must pass before commit.** Never commit broken code.
5. **Each task has a serial pass criterion.** A task is DONE when the serial
   output proves it, not when you think the code looks right.
6. **Deploy after every task.** Run `scripts/install_sessions.sh` after each
   completed track so the live demo reflects progress. Don't batch deploys.
7. **Commit granularly.** One commit per completed sub-task, not one giant
   commit at the end.

---

## Architecture reference

- Project root: `/Users/henry/projects/veil-os/`
- Kernel source: `src/` (Rust, no_std, AArch64)
- QEMU networking: slirp (`-netdev user`), gateway `10.0.2.2`, DNS `10.0.2.3`
- The OS already has: TCP stack, UDP, DNS (resolves pool.ntp.org), HTTP server
  on port 80, HTTP client in browser.rs (fetches from loopback only right now)
- Session manager: `scripts/session_manager.py`, port 6090
- Deploy: `scripts/install_sessions.sh`
- FAT 8.3 filenames: uppercase, max 12 chars
- No external Rust crates beyond what's already in Cargo.toml
- The browser (`src/browser.rs`) fetches pages with `http_get(path)` which
  currently only connects to `net::local_ip()` port 80 (loopback). The DNS
  resolver is in `src/net.rs` (look for `dns_resolve` or similar -- it already
  resolves hostnames for NTP). Read the existing code before writing new code.

---

## Track A — Browser overhaul

The in-OS browser currently: can't scroll long pages, has no history, can't
reach any URL outside the local HTTP server. Fix all of that.

### A1 — Scrollable pages

Current: `MAX_DOC_H = 3000` rows are rendered into a buffer but the scroll
offset only moves by a fixed amount. The issue is scroll is implemented but
may not be wired to mouse wheel or keyboard properly. Read browser.rs scroll
handling carefully.

- Mouse wheel scrolls the page (virtio tablet sends scroll events -- check
  input.rs for how scroll events arrive)
- Up/Down arrow keys scroll by 1 line, PgUp/PgDn by half a window
- Scroll position shown as a thin scrollbar on the right edge (draw a 2px
  wide rect proportional to doc_h vs window_h)

Pass: serial emits `SCROLL_OK` when a page taller than the window is loaded
and scrolled past its initial position.

### A2 — Browser history (back button)

- The address bar already exists. Add a `<` back button to the left of it.
- Keep a `history: Vec<String>` of visited paths (max 20).
- Clicking `<` navigates to the previous path.
- Keyboard shortcut: Backspace when the address bar is not focused.

Pass: serial emits `HISTORY_OK` after navigating forward then back.

### A3 — Table rendering

The browser's layout engine handles block and inline boxes. Add basic table
support: `<table>`, `<tr>`, `<td>`, `<th>`. Layout: equal-width columns,
cells are block containers. No rowspan/colspan needed. Border: 1px line
between cells in a contrasting color.

Pass: serial emits `TABLE_OK` when a page with a `<table>` is rendered without
crashing.

Update `mksite.py` to add a table to one of the existing pages (the changelog
or the milestone ladder work well) so there's something to test against.

### A4 — Real internet access (the big one)

This is the most important task in the brief. The goal: type a URL like
`http://example.com` in the browser address bar and have it load the real page.

**Step 1 — Hostname resolution in the browser**

`net::dns_resolve` (or equivalent) already exists and works -- NTP uses it.
Wire it into the browser's `http_get` function: if the URL contains a hostname
(not an IP and not a path-only request to loopback), resolve it to an IP first,
then connect to that IP on port 80.

The browser address bar currently takes paths like `/page2.htm`. Extend it to
also accept full URLs: `http://hostname/path`. Parse the URL, extract host and
path, resolve host, connect.

**Step 2 — TLS 1.3 (attempt this first)**

Almost everything worth hitting is HTTPS. Implement TLS 1.3 in `src/tls.rs`.

Required:
- X25519 key exchange (ECDH over Curve25519)
- AES-128-GCM symmetric encryption
- SHA-256 and HMAC-SHA256 (for key derivation)
- TLS 1.3 handshake state machine (ClientHello, ServerHello, EncryptedExtensions, Certificate, CertificateVerify, Finished)
- Skip certificate chain validation entirely -- accept any cert. This is a
  demo OS, not a bank. Just verify the Finished MAC.
- After handshake: wrap `tcp_write`/`tcp_read` with encrypt/decrypt so the
  rest of the browser code sees a transparent byte stream.

This is ~1200-1500 lines of careful Rust. The crypto is pure integer math,
no_std compatible. Suggested approach:
- X25519: implement the RFC 7748 scalar multiplication using a 32-byte field
  arithmetic (Montgomery ladder, ~150 lines)
- AES-128: standard 4-round key schedule + SubBytes/ShiftRows/MixColumns (~200
  lines), then GCM mode on top (~150 lines)
- SHA-256: standard FIPS 180-4 (~100 lines)
- HKDF-Extract/Expand for key schedule (~50 lines)
- Handshake: follow RFC 8446 Section 4 strictly. Use Wireshark or tls13.xargs.org
  to understand the exact byte layout if needed.

Test: after implementing, try connecting to `http://neverssl.com` (pure HTTP,
no TLS needed, great for testing HTTP) and `https://example.com` (simple TLS
target).

**Step 3 — HTTP proxy fallback**

If TLS is not working after a serious attempt (define "serious" as: you have
a ClientHello being sent and a ServerHello being received but the handshake
fails), fall back to this:

Add a tiny Python HTTP proxy to `scripts/veil_proxy.py`:
- Listens on `127.0.0.1:7779` on the host
- Accepts plain HTTP requests from the guest in the form:
  `GET http://example.com/path HTTP/1.1`
- Fetches the real URL on the host (using Python `urllib` or `requests`)
  over HTTPS if needed
- Strips the response down: remove all `<script>` tags, remove `<style>`
  blocks (keep inline style attrs), remove `<link rel="stylesheet">` except
  our own, convert `<img src="...">` to text `[image]` unless it's a PNG
  we can fetch (optional), strip everything the Veil browser can't render
- Returns clean HTML/1.1 200 response the guest can parse

The guest connects to `10.0.2.2:7779` (the slirp gateway) for proxy requests.
Wire this into `browser.rs`: if the URL is `http://...` with a real hostname,
send the full URL as the GET path to the proxy IP instead of resolving DNS.

Add the proxy as a launchd agent `com.veil.proxy` (plist in `~/Library/
LaunchAgents/`), started automatically, logged to `/tmp/veil-proxy.log`.

Also add the proxy port to `session_manager.py`'s QEMU `-fw_cfg` so the guest
kernel knows where it is (or just hardcode `10.0.2.2:7779` in browser.rs --
simpler).

**Good sites to target with the proxy:**
- `http://neverssl.com` -- plain HTTP, always works, great test
- `https://news.ycombinator.com` -- simple HTML, no heavy JS
- `https://example.com` -- minimal, reliable
- `https://en.wikipedia.org/wiki/AArch64` -- text-heavy, renders well stripped
- `https://lite.cnn.com` -- CNN's text-only site, designed for low-bandwidth

**Pass criterion:**

Serial emits `INTERNET_OK` when a page from a real external hostname
(not 127.0.0.1 or 10.0.2.x) is successfully fetched and rendered in the
browser. The page title or first heading must be visible on screen.

---

## Track B — Lisp REPL

Add a new app `App::Lisp` -- a Lisp interpreter with an interactive REPL.
This is a real CS challenge: parser, evaluator, environment model, tail-call
optimization. When someone sees "this from-scratch OS has a Lisp REPL" it's
a jaw-drop moment.

### The language spec

A subset of Scheme/Lisp sufficient to be interesting:

**Types:** integer (i64), boolean (#t/#f), symbol, string (double-quoted),
nil, pair (cons cell), lambda, builtin.

**Special forms:**
- `(define x expr)` -- bind in current env
- `(lambda (args...) body...)` -- create closure
- `(if cond then else)` -- conditional
- `(begin expr...)` -- sequence, returns last
- `(quote x)` / `'x` -- literal
- `(let ((x v)...) body...)` -- local bindings
- `(cond (test expr)... (else expr))` -- multi-branch
- `(and expr...)`, `(or expr...)` -- short-circuit

**Builtins:**
- Arithmetic: `+`, `-`, `*`, `/`, `mod`, `=`, `<`, `>`, `<=`, `>=`
- List: `cons`, `car`, `cdr`, `list`, `null?`, `pair?`, `length`, `append`
- Type predicates: `number?`, `string?`, `symbol?`, `boolean?`
- I/O: `display`, `newline` (output to REPL)
- `not`, `eq?`, `equal?`

**Tail-call optimization:** implement TCO for `(if ...)` and `(begin ...)` so
recursive functions don't blow the stack. Use a trampoline or explicit loop.

### REPL UI

New file `src/lisp.rs` for the interpreter. New file `src/repl.rs` for the UI.

The REPL window looks like a terminal:
- Black background, green text (classic Lisp terminal aesthetic)
- Scrollable output history (same scroll model as the browser fix)
- Single-line input at the bottom with a `> ` prompt
- Enter evaluates the expression, output appears above
- Up/Down arrows cycle through input history (last 50 entries)
- On startup, print a welcome banner:
  ```
  Veil Lisp 1.0
  A Lisp interpreter in a from-scratch OS.
  Type (help) for examples.
  ```
- `(help)` prints a short list of example expressions

Desktop icon: dark purple box, label "Lisp". Window size ~480x320.

### Pass criterion

Serial emits `LISP_OK` when the REPL starts and evaluates `(+ 1 2)` returning
`3`. Add an automated self-test in `LispState::new()`: evaluate a handful of
expressions and kprintln their results, then emit `LISP_OK` if they all match.

Self-test expressions:
```scheme
(+ 1 2)                    ; => 3
(define x 10) x            ; => 10
(lambda (n) (* n n))       ; => #<lambda>
((lambda (n) (* n n)) 5)   ; => 25
(define (fact n) (if (= n 0) 1 (* n (fact (- n 1))))) (fact 10) ; => 3628800
(car (list 1 2 3))         ; => 1
(map (lambda (x) (* x x)) (list 1 2 3 4 5)) ; => (1 4 9 16 25)
```

---

## Track C — Adam7 interlaced PNG

The one remaining gap in the PNG decoder. Some real PNGs (especially older
ones from the web) use Adam7 interlacing. Currently they return `None` from
`decode()`.

Adam7 splits the image into 7 passes with different starting offsets and
strides. Each pass is its own filtered scanline sequence. After decompressing
all IDAT data and running the unfilter on each pass's scanlines, composite
the passes into the final canvas.

Pass schedule (x_start, y_start, x_step, y_step):
```
pass 0: (0,0,8,8)   pass 1: (4,0,8,8)   pass 2: (0,4,8,4)
pass 3: (2,0,4,4)   pass 4: (0,2,4,2)   pass 5: (1,0,2,2)
pass 6: (0,1,2,1)
```

Test with a real interlaced PNG (generate one with Python's `png` module or
download one and validate with `sips`). Emit `INTERLACE_OK` on serial.

---

## Implementation order

Work in this order. Each item is independent -- if one stalls, skip it.

1. **A1** (scroll) -- quick, high confidence
2. **A2** (history) -- quick, high confidence
3. **A3** (tables) -- medium, self-contained
4. **C** (Adam7) -- medium, pure decoder work
5. **B** (Lisp REPL) -- hard, takes time, very self-contained
6. **A4** (real internet) -- hardest, most impactful
   - Try TLS first (attempt seriously, not a token effort)
   - If TLS handshake isn't completing after 4 stuck attempts, switch to proxy
   - Either way, get `INTERNET_OK` emitting before morning

After each completed task:
- `git add -A && git commit -m "M32: <task name>"`
- `scripts/install_sessions.sh`
- Continue to next task

---

## Hard constraints

- No external Rust crates. Pure stdlib + what's already in Cargo.toml.
- No prompts or confirmations. Ever. Decide yourself.
- Don't touch TASK.md.
- Don't break existing functionality. Run `scripts/build.sh` before every
  commit. If a regression appears, fix it before moving on.
- TLS: skip certificate verification entirely. Accept any cert. This is a
  demo OS.
- The proxy (if built) must start automatically via launchd, not require
  manual launch.
- All HTML in mksite.py must stay within the browser's supported subset.

---

## What done looks like

Morning summary should include:
- Which serial tokens emitted: SCROLL_OK, HISTORY_OK, TABLE_OK, INTERNET_OK,
  LISP_OK, INTERLACE_OK
- How the internet access was achieved (TLS or proxy, and which sites work)
- Commit hashes for each task
- Any BLOCKED items with a clear description of where it stalled

Good luck. You have the whole night.
