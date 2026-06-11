# Fable — M33 Session

Read this document in full before touching any file. You are running
autonomously. Make every decision yourself. Never pause for approval. Never
ask a question. If you hit a wall after 4 consecutive attempts on the same
problem, write a BLOCKED note in PROGRESS.md and move to the next task.

---

## Autonomy rules

1. Never ask for confirmation or approval. Make all decisions yourself.
2. Stuck rule: 4 failed attempts on the same problem → write BLOCKED in
   PROGRESS.md, move on.
3. Build must pass before every commit.
4. Each task has a serial pass criterion. Done = serial token emitted, not
   "code looks right."
5. Deploy after every completed task: `scripts/install_sessions.sh`
6. One commit per task.
7. Never break existing functionality. Run `scripts/build.sh` before every
   commit. Fix regressions before moving on.

---

## Architecture reference

- Project root: `/Users/henry/projects/veil-os/`
- Kernel source: `src/` (Rust, no_std, AArch64)
- QEMU networking: slirp (`-netdev user`), gateway `10.0.2.2`, DNS `10.0.2.3`
- The proxy (`scripts/veil_proxy.py`, launchd `com.veil.proxy`, port 7779)
  is running and the browser already uses it for external URLs
- Session manager: `scripts/session_manager.py`, port 6090
- noVNC page: look in the websockify/novnc install dir (find it with
  `find /usr/local /opt/homebrew -name "vnc.html" 2>/dev/null` or check
  NOVNC variable in session_manager.py)
- Deploy: `scripts/install_sessions.sh`
- No external Rust crates beyond what's already in Cargo.toml

---

## Task 1 — Browser audio (fix it actually playing)

The WebSocket PCM pipe is built and working (M28/M32). The session manager
drains the FIFO and forwards PCM to browser clients over WebSocket at
`/session/<id>/audio`. The noVNC page has `novnc_audio.js` wired in.

The problem: browser-side audio doesn't actually play. Diagnose and fix it.

Likely causes to check in order:
1. The Web Audio API context needs a user gesture to start (autoplay policy).
   The ♪ button click should create and resume the AudioContext -- verify
   this is actually happening and the context state is "running" after click.
2. PCM format mismatch -- the OS sends 16-bit signed stereo 44100Hz. The JS
   must create an AudioBuffer with the right sampleRate, 2 channels, and
   convert Int16 → Float32 before calling `source.start()`.
3. The WebSocket message framing -- verify the JS is receiving binary messages
   (not text) and reading them as ArrayBuffer.
4. Timing -- Web Audio needs buffers queued ahead of playback time. Use a
   small lookahead (0.1s) and schedule each chunk at `ctx.currentTime + lookahead`.

Fix `novnc_audio.js` (and any supporting JS) so that:
- Clicking ♪ starts the audio context
- PCM streams in and plays continuously without gaps
- Works in Chrome and Firefox

Pass: manually verify audio plays in the browser when the in-OS Audio app
plays TONE.WAV. Serial emits `AUDIO_BROWSER_OK` -- add this to the audio
app's playback path or have session_manager.py log it when a browser client
receives >10KB of PCM.

---

## Task 2 — Lisp persistence

The Lisp REPL resets every time the window closes. Fix it so the environment
persists across open/close cycles.

- On every successful `define`, serialize the top-level environment to
  `LISP.TXT` on the FAT16 disk (use the existing `fs::write_file` API --
  check how the editor saves files).
- On startup, if `LISP.TXT` exists, deserialize and restore the environment
  before showing the prompt.
- Format: simple s-expression dump is fine -- one `(define name value)` per
  line for atoms/numbers/strings. Lambdas can be serialized as their source
  form `(lambda (args) body)` if you stored the AST, or skipped (write a
  comment `; lambda <name> not serialized`).
- If `LISP.TXT` is corrupt or unparseable, silently start fresh (don't crash).

Pass: serial emits `LISP_PERSIST_OK`. Test: define a variable, close the REPL
window, reopen it, verify the variable is still bound.

---

## Task 3 — Lisp file I/O builtins

Add these builtins to the interpreter:

- `(read-file "FILENAME.TXT")` -- reads the named file from FAT16, returns
  its contents as a string. Returns `#f` if the file doesn't exist.
- `(write-file "FILENAME.TXT" string)` -- writes a string to a file on FAT16.
  Returns `#t` on success, `#f` on failure.
- `(list-files)` -- returns a list of filename strings on the disk.

Filenames are FAT 8.3 uppercase. Document this in the `(help)` output.

Pass: serial emits `LISP_IO_OK`. Self-test in `LispState::new()`:
  `(write-file "TEST.TXT" "hello")` then `(read-file "TEST.TXT")` returns
  `"hello"`.

---

## Task 4 — Desktop icon drag-to-reorganize

Users can't rearrange desktop icons. Add drag-and-drop reordering.

- Click and hold on an icon for 200ms to start dragging (distinguish from
  a tap/click which launches the app).
- While dragging, render the icon semi-transparent at the cursor position.
- On release, swap the dragged icon with whichever slot the cursor is nearest
  to, or insert into the nearest slot.
- Persist the order to `ICONS.TXT` on the FAT16 disk (one icon name per line).
  On startup, read `ICONS.TXT` and restore the order. If absent, use default.

Pass: serial emits `DRAG_OK` when an icon is dragged and dropped. The order
must survive a reboot (verified by reading ICONS.TXT content from the disk).

---

## Task 5 — TLS 1.3 from scratch

This is the main event. Implement TLS 1.3 in `src/tls.rs` so the browser can
connect directly to HTTPS sites without the proxy. The proxy stays as a
fallback for sites that break, but direct TLS should work for well-behaved
servers.

**Do not skip this. Make a serious attempt. The proxy was the right call
overnight under time pressure. Today there's time to do it right.**

### Crypto primitives needed

All pure Rust, no_std, no external crates. Implement in separate files or
modules:

**src/crypto/sha256.rs** -- SHA-256 (FIPS 180-4)
- `fn sha256(data: &[u8]) -> [u8; 32]`
- `fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32]`
- ~100 lines

**src/crypto/hkdf.rs** -- HKDF (RFC 5869) using HMAC-SHA256
- `fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32]`
- `fn hkdf_expand(prk: &[u8], info: &[u8], len: usize) -> Vec<u8>`
- ~50 lines

**src/crypto/x25519.rs** -- X25519 ECDH (RFC 7748)
- `fn x25519(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32]`
- `fn x25519_base(scalar: &[u8; 32]) -> [u8; 32]` (multiply by base point)
- Implement using the Montgomery ladder on the Curve25519 field (p = 2^255 - 19)
- Field arithmetic: 256-bit integers as [u64; 4] or [u32; 8], modular add/sub/mul/reduce
- ~250 lines. This is the hardest part. Take your time and get it right.
- Test: the RFC 7748 Section 6.1 test vectors MUST pass before you proceed.

**src/crypto/chacha20.rs** -- ChaCha20-Poly1305 (RFC 8439)
- Prefer this over AES-GCM -- simpler to implement correctly without hardware
  acceleration, constant-time naturally
- `fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64]`
- `fn poly1305_mac(key: &[u8; 32], msg: &[u8]) -> [u8; 16]`
- `fn chacha20poly1305_encrypt(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8>`
- `fn chacha20poly1305_decrypt(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>>`
- ~250 lines. Test against RFC 8439 test vectors before proceeding.

### TLS 1.3 handshake (src/tls.rs)

Follow RFC 8446. The cipher suite to negotiate: `TLS_CHACHA20_POLY1305_SHA256`.

**ClientHello:**
- TLS version: 0x0303 (TLS 1.2 compat) with supported_versions extension = 0x0304
- Random: 32 random bytes (use the timer tick XOR'd with some constants as a
  PRNG seed -- not cryptographically secure but fine for a demo OS)
- Cipher suites: [TLS_CHACHA20_POLY1305_SHA256 (0x1303)]
- Extensions: supported_versions, supported_groups (x25519), key_share
  (x25519 public key), signature_algorithms (ecdsa_secp256r1_sha256 +
  rsa_pkcs1_sha256), server_name (SNI)

**ServerHello parsing:**
- Extract server's x25519 key share
- Compute shared secret via x25519(our_private, server_public)
- Derive handshake keys using HKDF as per RFC 8446 Section 7.1

**After ServerHello:**
- Decrypt EncryptedExtensions, Certificate, CertificateVerify, Finished
  using the handshake traffic keys
- Skip certificate chain verification entirely -- accept any cert
- Verify the server Finished MAC (compute expected_finished using
  HMAC-SHA256 of the handshake transcript hash, verify it matches)
- Send client Finished

**Application data:**
- After handshake: wrap send/recv with ChaCha20-Poly1305 using the
  application traffic keys
- Expose: `fn tls_connect(host: &str, port: u16) -> Option<TlsConn>`
  where TlsConn has `fn write(&mut self, data: &[u8])` and
  `fn read(&mut self, buf: &mut [u8]) -> TcpRead`

**Wire into the browser:**
- In `browser.rs`, if the URL starts with `https://`, use `tls_connect`
  instead of plain `tcp_connect`
- The proxy remains for HTTP URLs to avoid double-fetching

**Testing strategy:**
- First test X25519 with RFC 7748 vectors (offline, no network needed)
- Then test ChaCha20-Poly1305 with RFC 8439 vectors
- Then attempt a TLS handshake against `example.com:443` -- capture the
  raw bytes with `kprintln!` to debug if the handshake fails
- Use the serial log to trace exactly where the handshake diverges
- `tls13.xargs.org` and Wireshark packet captures are useful references
  for exact byte layout if needed

**Pass criterion:**
Serial emits `TLS_OK` when a complete TLS 1.3 handshake with `example.com`
succeeds and application data is exchanged (i.e., an HTTP GET returns a
200 response). Then emit `HTTPS_OK` when `https://example.com` renders in
the browser.

If after a serious attempt (X25519 vectors pass, ChaCha20 vectors pass,
handshake is being attempted but server rejects) you cannot get a working
handshake, document exactly where it fails in PROGRESS.md and leave the
proxy as the internet mechanism. Do not spin forever.

---

## Implementation order

1. Task 1 -- browser audio (quick, high value)
2. Task 2 -- Lisp persistence (quick)
3. Task 3 -- Lisp file I/O (quick)
4. Task 4 -- icon drag (medium)
5. Task 5 -- TLS (hard, take as long as needed)

After each task:
- `git add -A && git commit -m "M33: <task name>"`
- `scripts/install_sessions.sh`

---

## What done looks like

PROGRESS.md should end with a summary listing which serial tokens fired:
AUDIO_BROWSER_OK, LISP_PERSIST_OK, LISP_IO_OK, DRAG_OK, TLS_OK, HTTPS_OK

And for TLS: exactly which sites load natively vs through the proxy.
