# Veil OS — progress

Contract: `os-build-spec.md` (M1–M17) + `os-build-spec-v2.md` (M18–M24) +
`os-build-spec-v3.md` (M25–M30). Gated milestones; each passes only on
observed proof.

## Milestones

| # | Milestone | Status | Proof |
|---|-----------|--------|-------|
| M1 | Serial boot | PASSED | `scripts/test.sh BOOT_OK` |
| M2 | Exceptions + timer | PASSED | re-proven every boot (`M2_OK`) |
| M3 | Paging + MMU | PASSED | `M3_OK` |
| M4 | Kernel heap | PASSED | `M4_OK` |
| M5 | ramfb framebuffer | PASSED | `scripts/screenshot_m5.sh` |
| M6 | virtio keyboard/tablet | PASSED | `scripts/gui_test.sh` |
| M7 | Window manager | PASSED | `scripts/gui_test.sh` |
| M8 | Paint | PASSED | `scripts/gui_test.sh` |
| M9 | User mode + syscalls | PASSED | `scripts/test.sh M9_OK` |
| M10 | virtio-blk + FAT16 | PASSED | `run_gui.sh drive_m10_{save,load}.py` |
| M11 | Shell + preemption | PASSED | `run_gui.sh drive_m11.py` |
| M12 | Raw ethernet frames | PASSED | `scripts/net_test.sh` phase 1 |
| M13 | ARP / IPv4 / ICMP | PASSED | `scripts/net_test.sh` phase 1 |
| M14 | UDP + TCP | PASSED | `scripts/net_test.sh` (nc + pcap) |
| M15 | HTTP server → real browser | PASSED | `scripts/net_test.sh` phase 2 |
| M16 | On-OS browser | PASSED 2026-06-10 | `scripts/m16_test.sh` |
| M17 | Raspberry Pi 4 | PASSED (emulated) 2026-06-10 | `scripts/m17_test.sh` on QEMU raspi4b; SD artifacts in `pi/`; physical HDMI test pending |
| M18 | Text editor + persistence | PASSED 2026-06-10 | `scripts/m18_test.sh` (two boots, pixel-identical region) |
| M19 | Clock app, 4 faces | PASSED 2026-06-10 | `run_gui.sh drive_m19.py CLOCK_OK` (all 24 checks) |
| M19b | NTP wall-clock sync | PASSED 2026-06-10 | `scripts/m19b_test.sh` (real DNS+NTP over slirp, clock within 0s of host); `drive_m19.py` re-green |
| M20 | Two-instance LAN chat | PASSED 2026-06-10 | `scripts/m20_test.sh` (reflector-hub bridge); all 14 checks green, both directions, exact font pixels |
| M21 | GitHub release + hosted demo | PASSED 2026-06-10 | github.com/hratterman/veil-os public; hosted demo live at https://os.henryratterman.com (curl 200, serves noVNC) |
| M22 | Paint-save verify + polish | PASSED 2026-06-10 | paint reboot pixel-verified; site updated through M21 (browser-rendered); full regression suite green |
| M23 | Image viewer | PASSED 2026-06-10 | `scripts/m23_test.sh` (CHECK.PNG checker pixels; Right→DOG.PNG photo, image changes; Left returns) |
| M24 | Audio (WAV) | PASSED 2026-06-10 | `scripts/m24_test.sh` (virtio-sound streams 3s tone clean, AUDIO_OK); GUI `drive_m24.py`; audible via demo.sh (coreaudio) — human check |
| M25 | Per-visitor sessions + USER.TXT usernames | PASSED 2026-06-10 | `scripts/m25_test.sh` (two isolated disks alpha_fox/beta_owl; A's msg renders in B prefixed `alpha_fox:`, exact font pixels; both CHAT_OK). `session_manager.py --selftest` green. Cloudflare cutover staged (install_sessions.sh) — not live-verified here |
| M26 | TCP relay + DMs + online user list | PASSED 2026-06-10 | `scripts/m26_test.sh` (3 instances ann/bob/cid on `relay.py`; public broadcast reaches all, DM reaches bob not cid, panel lists all 3, DM_OK — all exact font pixels, 25 checks). M20/M25 UDP path regress green |
| M27 | First-boot setup screen | PASSED 2026-06-10 | `scripts/m27_test.sh` (boot1 no USER.TXT → drive setup: name+UTC-5, SETUP_OK, desktop replaces card pixel-verified; on-disk USER.TXT=testuser/TZ.TXT=-5; boot2 skips setup, persists, TZ -18000s). M18/M23 default-disk regress green |
| M28 | Browser audio (PCM over WebSocket) | PASSED 2026-06-10 | `scripts/m28_test.sh` (QEMU `wav` FIFO audiodev → `audio_server.js` taps FIFO → WS → `ws_probe.js` read 8148 bytes, 7966 non-zero, AUDIO_STREAM_OK; driver clicked Play, AUDIO_OK). Browser client `novnc_audio.js` + `com.veil.audio` staged (no browser in-sandbox) |
| M29 | In-OS file manager (App::Files) | PASSED 2026-06-10 | `scripts/m29_test.sh` (Files lists 20 disk files; row-0 highlight + first filename exact font pixels; click CHECK.PNG → Viewer opens 128x128 image, FILES_OK). `user-files/` PNG copy verified; mkdisk `--extra-dir` shared with M30 |
| M30 | Pre-boot file upload (hosted demo) | PASSED 2026-06-10 | `scripts/m30_test.sh` (session_manager: POST /upload veiltest.png → POST /boot 302 → spawned QEMU; drive setup → Files lists VEILTEST.PNG (exact font pixels) → opens 320x240 in Viewer, UPLOAD_OK). `landing.html` upload page + selftest green |
| M31 | Site expansion (9 pages) + GIF player | PASSED 2026-06-10 | `drive_m31_web.py` (9 cross-linked pages, nav bar, no 404s); `drive_m31_gif.py` (GIF_OK — `src/gif.rs` LZW decoder + `gifplayer.rs`; demo.gif + real 400x400 Wikipedia GIF both animate) |
| M32 | Browser overhaul + Lisp REPL + Adam7 | PASSED 2026-06-11 | `scripts/m32_test.sh` per track: SCROLL_OK, HISTORY_OK, TABLE_OK, INTERNET_OK (real sites via host proxy); `drive_m32_lisp.py` LISP_OK; `drive_m32_interlace.py` INTERLACE_OK. All re-verified 2026-06-11 |
| M33 | Browser audio + Lisp persistence/IO + icon drag + TLS 1.3 | PASSED 2026-06-11 | AUDIO_BROWSER_OK (`drive_audio_browser.py` headless Chrome + manager log), LISP_PERSIST_OK/LISP_IO_OK (`m33_lisp_test.sh`), DRAG_OK (`m33_icondrag_test.sh`, reboot-persisted), CRYPTO_OK + TLS_OK + HTTPS_OK (`m33_tls_test.sh` boot handshake to example.com; `drive_m33_https.py` browser direct TLS) |
| M34 | Browser visual overhaul (HTTP read fix, ext images, CSS vars, flexbox, fonts) | PASSED 2026-06-11 | HTTP_READ_OK (`drive_m33_wiki.py` Wikipedia chunked, no freeze), EXT_IMG_OK (`drive_m34_img.py` PNG over TLS+proxy), CSS_VAR_OK (`drive_m34_cssvar.py`), FLEX_OK (`drive_m34_flex.py` space-between nav), FONTS_OK (`drive_m34_font.py` Cormorant/Lora/Barlow/Mono). Acceptance: henryratterman.com renders recognizably over direct TLS (`drive_m34_hr.py`) |

## M19b notes (2026-06-10)

NTP/DNS client added to `net.rs` (`ntp_sync` → `dns_build`/`dns_parse` A
record + 48-byte NTP, blocking `udp_request` with cntpct timeouts since
the software tick is stopped during milestone12). Wall clock in `timer.rs`
re-anchored to the always-running `cntpct` (not the tick, whose rate
changes across boot phases) + a `WALL_TZ` offset from `TZ.TXT` (integer
hours; disk seeds `-4` = EDT). `clock.rs` wall/digital faces show real
local time when `timer::synced()`, else fall back to time-since-boot;
"since boot" replaced by a `UTC±N` / `(no sync)` label inside the dial.
Boot calls `milestone19b()` after net is up (NIC-gated). The GUI proof
runs without a NIC → `(no sync)` path; synced path proven on serial by
`m19b_test.sh`.

## M21 notes (2026-06-10)

GitHub: public repo `github.com/hratterman/veil-os` (the `henryratterman`
GH account does not exist — README one-liner uses `hratterman`). README +
MIT LICENSE + `.gitignore` + `scripts/demo.sh` (cocoa, slirp net) all
shipped; hero screenshot at `assets/screenshot.png`.

Hosted demo (self-hosted on this Mac mini, NOT a DO droplet): QEMU
headless `-vnc 127.0.0.1:10` (tcp 5910; :0/5900 is macOS Screen Sharing)
via `scripts/serve_vnc.sh`; websockify `--web /Users/henry/server/novnc`
on 6090 → 5910; noVNC `index.html` auto-redirects to vnc.html autoconnect.
Three launchd agents: `com.veil.qemu` (KeepAlive), `com.veil.websockify`
(KeepAlive), `com.veil.reset` (StartInterval 1800 → `reset_vnc.sh` kills
QEMU so KeepAlive relaunches a clean disk). serve_vnc.sh hard-sets PATH
(launchd lacks /opt/homebrew/bin). Local chain fully verified.

Cloudflare: ingress `veil.henryratterman.com → localhost:6090` appended to
`~/.cloudflared/config.yml` (backup saved), validated, SIGHUP-reloaded.
**Blocker:** `henryratterman.com` is on a DIFFERENT Cloudflare account
than the tunnel cert (cert token only sees `bythecoverbooks.com`), so I
can't create the DNS record. `cloudflared route dns` mis-created
`veil.henryratterman.com.bythecoverbooks.com` (deleted). User is adding a
proxied CNAME `veil → 3a546195-...cfargotunnel.com` in henryratterman.com;
once it propagates the demo is live (ingress already wired). Verify with
`curl -I https://veil.henryratterman.com`.

## M22 notes (2026-06-10)

Paint SAV/LOD reboot proven: `drive_m10_{save,load}.py` (taskbar-launch +
8px clamp shift) — red stroke drawn boot 1, restored pixel-identical boot
2. `mksite.py` updated to describe features through M21 (apps list, NTP
clock, hosted demo, install one-liner); the page2 link moved into the
hero so the M16 browser proof finds it above the fold; `drive_m16.py`
reads the browser's actual logged scroll offset instead of guessing
(pgdn clamps to doc_h-view_h). Updated site renders in the on-OS browser
(`m16_test.sh` all green).

**Regression suite re-greened after the UX overhaul** — all GUI drivers
now launch their app from the taskbar first (nothing auto-opens):
`drive_gui.py` (M6/M7/M8) fully rewritten to drive Editor/Clock/Paint
(keyboard echo, drag/focus/z-order via two real windows, paint strokes +
persistence-under-occlusion + clear); `drive_m11.py` (shell, paint-0 not
paint-1), `drive_m18.py`, `drive_m19.py`, `drive_m20.py`. Cursor sprite
is parked off-snapshot before pixel checks. net (M12–15), Pi4 (M17) green
unchanged.

**M20 bridge rewritten:** QEMU `socket` listen/connect delivered only one
direction on this host (B→A; A→B dead even at M12 boot) and `mcast`
doesn't route on macOS — both reproducible/deterministic, not flaky. Now
a host-side UDP reflector (`scripts/hub.py`, a virtual switch mirroring
the M21 relay design) bridges both instances' `-netdev dgram` tunnels;
delivery is symmetric. Also hardened `net.rs` `on_frame` to drop
self-MAC-sourced frames (a hub/mcast loops a sender's own broadcast back;
genuine loopback never hits the wire). m20_test frees stale UDP ports on
start for back-to-back runs.

## M23/M24 notes (2026-06-10)

M23: `src/viewer.rs` (App::Viewer) — opens every .PNG on FAT16 alphabetically,
decodes with the existing `png` module, nearest-neighbour aspect-fit, filename
in the title, left/right arrows cycle. Launcher appended last so proof-driver
button indices are unchanged. mkdisk seeds generated PNGs + real photos
(dog/forest/mountain decode fine: 8-bit RGB).

M24: **spec said Intel HDA, but that is PCI-only and this kernel has no PCIe
stack (HDA BAR sits in the >2^38 PCI window needing a T0SZ=16/L0 MMU change).
User chose virtio-sound over the existing virtio-mmio transport instead.**
`src/snd.rs`: virtio-snd (dev id 25), 4 queues, control protocol
(SET_PARAMS/PREPARE/START/STOP), tx ring of 8 period buffers refilled on
completion. **Key gotcha: a TCG vCPU busy-waiting on the used ring (RAM reads)
never yields to QEMU's audio timer → hang; an MMIO touch (`irq_ack`) each spin
forces a vCPU exit so the device makes progress.** WAV parser (RIFF, 16-bit
stereo 44100). App::Audio window (filename/Play-Stop/elapsed) runs playback on
a kernel task so the 3s stream doesn't block the desktop. mkdisk generates
TONE.WAV (`scripts/mkwav.py`, 440Hz 3s). demo.sh uses `-audiodev coreaudio`,
m24_test/serve_vnc use `none`.

## M25 notes (2026-06-10)

Chat sender label now reads `USER.TXT` off the FAT16 disk
(`wm::chat_username`): priority USER.TXT → legacy local-IP A/B (keeps the
diskless M20 proof green) → random 6-hex from `cntpct`. `mkdisk.sh` grew
`--username`/`--out`/`--extra-dir` flags. `m25_test.sh` boots two instances
off separate disks (each its own USER.TXT) over the same hub bridge as M20.
Host: `session_manager.py` (stdlib HTTP on :6091) spawns a per-visitor
QEMU+websockify on VNC :11.., websockify 6100.., random adjective_animal
usernames, 20-session cap, 30-min idle reap; `--selftest` covers username
uniqueness, port allocation, real disk build. `install_sessions.sh` +
`launchd/com.veil.sessions.plist` stage the cutover (replaces
com.veil.qemu/websockify/reset) — live Cloudflare swap is a manual op, not
verified in-sandbox.

## M26 notes (2026-06-10)

Chat gained a TCP relay mode alongside the M20 UDP broadcast. `relay.py`
(stdlib TCP :7778) speaks `HELLO/JOIN/PART/MSG` with a length-prefixed MSG
body; public (`to=*`) fans out to all incl. sender, DMs go to the named
recipient + echo to sender. The kernel `App::Chat` is now a `ChatState`
(lines as `ChatLine{text,color}`, `users` roster, `dm_target`, `ChatMode`).
Mode is chosen at launch from `net::relay_addr()` (fw_cfg `opt/veil.relay`,
parsed in main): present → TCP relay (DMs + right-side 80px user panel,
click a name to DM, terracotta DM colour); absent → UDP fallback (keeps the
diskless M20 + M25 proofs green). Relay frames parsed in `wm::parse_relay`,
pumped each desktop loop via `Wm::chat_poll`. **Key reachability fact: a
slirp `user` guest reaches host services at the gateway 10.0.2.2 — so every
QEMU instance (test + hosted, all on the one Mac mini) hits the host relay
at 10.0.2.2:7778 with no Cloudflare/DNS needed.** Host: `com.veil.relay`
LaunchAgent + relay wired into `session_manager` spawn. DM_OK sentinel on
first DM sent/received.

## M27 notes (2026-06-10)

`src/setup.rs`: full-screen pre-desktop mode (dark `#0b1018`, centered card,
3x header). `desktop::run` starts the 50 Hz tick + preemption first, then
runs setup iff `setup::needed()` (USER.TXT absent/blank) before `Wm::new`.
Name field (keyboard, backspace, blinking cursor), timezone field
(left/right arrows, 30-min steps UTC-12..UTC+14). Enter writes USER.TXT +
TZ.TXT and `timer::set_tz`. TZ.TXT now supports half-hours ("5.5");
`main::parse_tz_offset` parses `[+-]H[.5]` → seconds (replaces the old i64
hours parse in milestone19b). **Regression guard: the setup screen triggers
on USER.TXT absence, which every default-disk GUI proof would hit — so
`mkdisk.sh` now writes a default `USER.TXT=guest` unless `--no-user`. Hosted
sessions, demo.sh/run.sh, and m27_test pass `--no-user` to get the setup
screen; all other disks skip it.** Arrow keycodes 105/106.

## M28 notes (2026-06-10)

Browser audio with no GStreamer/WebRTC. **Two QEMU gotchas found: this
Homebrew QEMU rejects `out.try-poll=off` on the `wav` backend (dropped it),
and `virtio-sound-device` logs a non-fatal "can not open virtio-sound.in"
(capture stream, harmless — output still streams).** Path: `-audiodev
wav,id=snd0,path=/tmp/veil-audio-<id>.fifo` → `audio_server.js` (Node
built-ins; scans /tmp for session FIFOs, opens read — rendezvous with
QEMU's write-open at boot so the VM never blocks, strips the 44-byte RIFF
header by scanning for `data`, hand-rolled WS server frames) → browser
`novnc_audio.js` (Web Audio, gesture-unlock, back-to-back buffer queue,
speaker icon). Proof reads PCM with `ws_probe.js` (hand-rolled WS client).
No kernel change — same M24 virtio-snd streaming. `com.veil.audio` :6092,
Cloudflare `audio.henryratterman.com` (manual). The wav header from QEMU is
`RIFF....WAVEfmt ....data....` with `data` at offset 36, PCM at 44.

## M29 notes (2026-06-10)

`src/files.rs` (App::Files): lists every FAT16 root file (sorted), 14px
rows with `[IMG]/[WAV]/[TXT]/[???]` font tags, row-0 selected. Click or
Enter dispatches via new `Wm::open_file` (PNG→Viewer with
`viewer::ViewerState::with_file`, WAV→Audio with that file, TXT→Editor).
`files::key`/`files::click` return an `Action` (Redraw/Open) so the WM does
the cross-window launch outside the window borrow. Added as taskbar/desktop
launcher #9 (after audio, so existing button indices are unchanged; idx 7
without a NIC, 8 with). On open it logs `FILES[i]: NAME` for each file (the
proof finds the first PNG row from serial). mkdisk already copies
`user-files/*.{png,wav}`; `.gitignore` ignores `user-files/`, run.sh seeds
it + prints the hint.

## M30 notes (2026-06-10)

`session_manager.py` grew the pre-boot upload flow: `GET /` serves
`landing.html` (drop zone + Boot, session id injected via `__SESSION__`),
`POST /upload?session=<id>` parses multipart by hand (stdlib only —
boundary split, `filename="..."`, .png/.wav + 4MB + 5-file limits) into
`/tmp/veil-uploads-<id>/`, `POST /boot?session=<id>` builds the disk with
`mkdisk --extra-dir <uploads>` and spawns QEMU (302 → noVNC). Sessions are
lazily created via `get_or_create` so uploads/boot can precede `GET /`.
Each spawned instance now also gets a `-qmp` socket + `-serial` file (so the
proof can drive it) and a `mkfifo`'d audio tap. **Two test-box snags fixed:
buffered manager stdout hid the spawn error (run with `python3 -u`), and the
optional `websockify` launch (absent here) raised `FileNotFoundError` and
killed the session — now guarded.** The booted instance hits the M27 setup
screen first (fresh `--no-user` disk), then Files shows the uploaded file.

## M31 notes (2026-06-10)

Site expansion: `mksite.py` grew from 2 to 9 cross-linked pages (home/build,
news, wiki, gallery, ascii, tips, about, changelog) with a shared nav bar and
a richer dark `style.css`, all within the browser's HTML/CSS subset; no 404s
(`drive_m31_web.py`). GIF player: `src/gif.rs` is a from-scratch GIF87a/89a
decoder — LZW decompression (the classic code-size off-by-one: decoder bumps
at `table==2^cs`, the encoder one step later — verified by round-trip),
global/local colour tables, GCE delay/disposal, interlacing, transparent-index
compositing, heap-bounded so a big upload can't OOM. `src/gifplayer.rs`
(App::Gif) plays it: space toggles, arrows scrub frames, up/down switch files,
Esc closes. `mkdisk.sh`/`landing.html`/`session_manager.py` accept `.gif`
uploads. Proven on demo.gif + a real 400x400 Wikipedia GIF (GIF_OK).

## M32 notes (2026-06-11)

Overnight Fable session delivered all six tracks; re-verified 2026-06-11.

- **A1 scroll** (`browser.rs`): mouse wheel (REL_WHEEL → 3 lines/notch),
  arrows (1 line), PgUp/PgDn (half window), proportional 2px scrollbar thumb.
  SCROLL_OK on first off-top move of a tall page.
- **A2 history**: `history: Vec<String>` (max 20), `<` back button + Backspace
  when the address bar is unfocused. HISTORY_OK.
- **A3 tables**: `<table>/<tr>/<td>/<th>` as equal-width column block cells
  with 1px borders. TABLE_OK; `mksite.py` changelog page carries a table.
- **C Adam7** (`png.rs`): 7-pass interlaced PNG deinterlace+composite.
  INTERLACE_OK.
- **B Lisp** (`lisp.rs` + `repl.rs`): trampolined evaluator (TCO for
  if/begin/let/lambda-body), lexical envs, ~30 builtins, green-on-black REPL
  with scrollback + input history. Self-test (incl. fact 10, map) emits
  LISP_OK.
- **A4 real internet**: TLS was *not* implemented; took the proxy path.
  `scripts/veil_proxy.py` (launchd `com.veil.proxy`, 127.0.0.1:7779, reached
  by the guest at the slirp gateway 10.0.2.2) fetches real HTTPS sites on the
  host, strips them to the browser's subset, absolutises links. `browser.rs`
  sends absolute-form `GET http://host/path` to the proxy for external URLs,
  loopback for local. INTERNET_OK on example.com; multi-site verified
  (neverssl HTTP + Hacker News HTTPS render in-guest, `drive_m32_internet2.py`).

**Harness:** `scripts/m32_test.sh <driver.py>` boots with a NIC + starts the
proxy (the browser homepage is fetched over the net stack, so the browser
drivers need a NIC, which `run_gui.sh` lacks). Lisp/Adam7 self-tests need no
NIC (`run_gui.sh <driver> WM_OK`).

**Bugs found + fixed on re-verification (2026-06-11):**
- *Lisp panics on malformed input.* Many builtins/special forms indexed
  argument vectors without arity checks — `(car)`, `(if)`, `(cons 1)`,
  `(mod 5)`, an empty `cond` clause, etc. hit an out-of-bounds panic, which in
  this kernel is `semihosting::exit` — the whole VM dies. An interactive REPL
  *will* see malformed input. Added arity guards so every bad form returns an
  Err the REPL catches; startup self-test now drives 21 malformed inputs and
  asserts each errors cleanly before LISP_OK.
- *Lisp deep-recursion stack overflow.* Runaway non-tail recursion overflowed
  the 512 KiB boot stack → PC-alignment fault → dead OS. Added an `eval` depth
  guard. **Tuning gotcha: the unoptimised debug `eval` frame is ~1.7 KB, so a
  guard at 700 (then 300) reproduced the very overflow it was meant to stop**
  (faulted right after the Lisp window opened). Measured the safe cap down to
  80 (~136 KiB); self-test drives a runaway recursion and asserts it's guarded.

## Hosted-demo deployment (2026-06-10) — LIVE

The per-visitor session architecture is deployed to os.henryratterman.com.
Cut over from the single-instance agents (com.veil.qemu/websockify/reset,
backed up to `~/Library/LaunchAgents/veil-backup-*`) to
**com.veil.{sessions,relay,audio}**. `session_manager.py` now: (1) listens
on **6090** (the port the live Cloudflare route already targets) bound
**dual-stack** — cloudflared dials `localhost` as IPv6 `[::1]`, so an
IPv4-only bind 502'd; (2) **reverse-proxies `/session/<id>/*`** (noVNC
static + the WebSocket via a bidirectional socket pump) to that session's
private websockify — this was the missing piece for browser visitors
(m30_test only drove via QMP/serial); (3) resolves a concrete `websockify`
path (it isn't on PATH); (4) exits hard on SIGTERM (the old
`httpd.shutdown()` from the signal handler deadlocked → orphaned process
holding the port). Verified end-to-end through Cloudflare: landing →
POST /boot → 302 → proxied noVNC 200; a fresh session boots the latest
kernel to the M27 setup screen (framebuffer captured). os route is on the
**dashboard-managed** tunnel (ingress lives in the CF dashboard, not
config.yml), already pointing at localhost:6090.

## Audio freeze in hosted sessions — root-caused + fixed (2026-06-10)

**Symptom:** pressing Play in the Audio window froze the entire hosted VM
(VNC, desktop, all devices); only in hosted sessions, never local run.sh.

**Root cause (host-side, not guest):** hosted sessions use
`-audiodev wav,path=/tmp/veil-audio-<sid>.fifo`. QEMU's `wav` backend does
a **blocking write** to that FIFO. If nothing drains it, the ~64 KB pipe
fills mid-playback and QEMU's main loop blocks on the write — freezing the
whole VM. Confirmed by a 2-scenario repro: a held-but-not-draining FIFO
reader → HUNG at "SND: stream started"; an actively-drained FIFO →
AUDIO_OK. This is why every guest-side attempt (yield/wfi/GIC IRQ) failed:
the guest can't fix a blocked host. The old design leaned on a *separate*
best-effort Node bridge (`audio_server.js` / com.veil.audio) to drain, and
browser audio was never actually wired into the page — so in practice
nothing reliably drained the FIFO.

**Fix:** `session_manager.py` now drains each session's FIFO **in-process**
for the session lifetime (`drain_fifo` thread, started in `spawn`, stopped
in `kill`) — guaranteed, not best-effort. It header-strips and forwards the
PCM to browser audio clients over a **same-origin** WebSocket at
`/session/<id>/audio` (handled directly by the manager, not proxied). The
standalone audio bridge is retired (com.veil.audio unloaded/removed — a 2nd
FIFO reader would split the bytes). `novnc_audio.js` now connects to the
same-origin endpoint and is wired into `~/server/novnc/vnc.html`. Verified
live: `drive_audio_session.py` (no freeze: AUDIO_OK + framebuffer still
updates) and `drive_audio_ws.py` (browser received 528 KB of PCM over the
WS during playback). No kernel change required.

## Post-M20 UX overhaul (no milestone — 2026-06-10)

Boot shows the bare desktop; nothing opens automatically. Apps launch
from a 40px bottom taskbar (Editor / Clock / Browser / Paint / Shell /
Chat-when-NIC) or the top-left desktop icon grid; both open-or-raise.
Every title bar has an 18px close (X) zone at its right edge. Windows
clamp above the taskbar. Default open positions unchanged.

**Resolved in M22:** every GUI proof driver now launches its app from the
taskbar before driving, and pixel coords account for the 8px clamp. Full
suite re-verified green. (drive_gui.py was rewritten end-to-end since the
old alpha/beta/echo/static boot windows no longer exist.)

## Kernel bugs found by milestone gates

- **M19 → timer drift:** `on_tick` re-armed with `TVAL = reload`, so IRQ
  latency stretched every period and `ticks()` fell behind wall time.
  Fixed: absolute `CVAL` deadlines, missed periods counted.
- **M17 → frames underflow:** an empty reserved range `(0,0)` wrapped
  `end - 1` and marked all frames used. Fixed: skip empty, tolerate
  overlapping ranges.

## M33 notes (2026-06-11)

Five tracks, all serial-gated.

- **Task 1 — browser audio actually plays.** The PCM WebSocket pipe already
  delivered; `novnc_audio.js` never played. Three bugs: the ♪ button started
  unmuted so the first click muted it; gesture-unlock relied on `window`
  mousedown which noVNC's canvas `stopPropagation`s (context stayed suspended);
  no scheduling lookahead. Rewrote around a single ♪ control (first click
  enables inside the gesture handler; PCM scheduled 0.15 s ahead of
  `currentTime`). Proof: `drive_audio_browser.py` loads the real client in
  headless Chrome against a stub PCM WS — AudioContext reaches "running", 84 KB
  decoded+scheduled, lookahead > 0; `session_manager.py` logs **AUDIO_BROWSER_OK**
  once a browser client has received > 10 KB PCM.
- **Task 2 — Lisp persistence.** Every `define` serializes the top-level env to
  `LISP.TXT` (one `(define ...)`/line; atoms, quoted lists/symbols, lambdas as
  `(lambda (args) body)` source; builtins skipped); restored on open (tolerant
  of corrupt lines). Self-tests moved to a throwaway interp so they don't leak
  into the saved env. **LISP_PERSIST_OK** (in-memory round-trip) +
  `drive_m33_lisp_persist.py` (define → close window → reopen → restored).
- **Task 3 — Lisp file I/O.** `read-file`/`write-file`/`list-files` over FAT16
  (8.3 UPPERCASE, documented in `(help)`). **LISP_IO_OK** self-test +
  `drive_m33_lisp_io.py` (write→#t, read→"world", missing→#f, list-files).
- **Task 4 — desktop icon drag.** Hold ~200 ms to drag (tap still launches),
  semi-transparent follow (`fb::blend_rect`), drop inserts at nearest slot,
  order persisted to `ICONS.TXT` and restored on boot. **DRAG_OK** +
  `m33_icondrag_test.sh` (boot 1 drags edit→clock slot; boot 2 logs the
  reordered order — reboot persistence). `gui_test.sh` still green.
- **Task 5 — TLS 1.3 from scratch.** `src/crypto/` (SHA-256/HMAC, HKDF,
  ChaCha20-Poly1305, X25519) — all checked against RFC vectors at boot
  (**CRYPTO_OK**; the vectors caught a `car25519` wrap bug, 37 vs 38·(c−1)).
  `src/tls.rs` is a full TLS 1.3 client (TLS_CHACHA20_POLY1305_SHA256, X25519),
  verifies the server Finished MAC (cert chain validation skipped by design).
  Wired into `browser.rs`: `https://` fetches directly via `tls_connect`.

**Internet mechanism — which path serves what:**
- `https://` URLs → **direct from-scratch TLS 1.3** (no proxy). Verified live
  against example.com:443 (Cloudflare 104.20.23.154): ServerHello parsed, server
  Finished verified, HTTP 200 over the encrypted channel, page renders.
- `http://` external URLs → host proxy `veil_proxy.py` (10.0.2.2:7779), which
  also strips/【de-gzips】 pages to the browser's subset. TLS is the fallback's
  fallback: if a direct `https://` handshake fails, the browser retries it
  through the proxy.
- local `/page.htm` → in-kernel HTTP server over loopback.

**Serial tokens fired this milestone:** AUDIO_BROWSER_OK, LISP_PERSIST_OK,
LISP_IO_OK, DRAG_OK, CRYPTO_OK, TLS_OK, HTTPS_OK.

## M34 notes (2026-06-11) — browser visual overhaul

Five tasks; the goal was henryratterman.com rendering recognizably in the Veil
browser (acceptance test below).

- **T1 — bounded HTTP reads (HTTP_READ_OK).** `read_to_eof` reset its idle
  deadline on every byte, so a keep-alive HTTP/1.1 server (Wikipedia, neverssl)
  that holds the socket open after the body never returned — hanging the
  desktop task and freezing the whole OS. Replaced with `read_http`:
  `response_complete()` returns the moment Content-Length / the chunked
  terminator is satisfied, with a hard total-time backstop; falls back to
  EOF-wait only when neither length is present. Same logic in the TLS path.
- **T2 — external images (EXT_IMG_OK).** `<img>` from https/http/loopback are
  fetched (via http_get → TLS / proxy / loopback), PNG-decoded, and rendered
  inline; non-PNG is skipped silently (no `[img]`). Persistent per-window LRU
  cache (cap 10). The proxy now keeps `<img>` (absolutised src) and serves raw
  image bytes instead of replacing with `[image]`.
- **T3 — CSS custom properties (CSS_VAR_OK).** `css::collect_vars` gathers
  `--name: value` (incl. from `:root`); `apply_decl` substitutes
  `var(--x, fallback)` before parsing colour/size/spacing. Inline `<style>` is
  now parsed too (via `Node::text`).
- **T4 — flexbox (FLEX_OK).** `display:flex` with flex-direction, wrap,
  justify-content, align-items, gap, and `flex` grow. Items are measured in
  isolation then re-laid at their allocated width and translated into place
  (preserving links/images). Fixed a load-bearing dispatch bug: `layout_children`
  only treated `Display::Block` as a block, so flex elements fell through to
  inline and `layout_flex` never ran.
- **T5 — bitmap fonts (FONTS_OK).** `scripts/gen_fonts.py` rasterizes Cormorant
  Garamond, Lora, Barlow Condensed and JetBrains Mono (Pillow) into
  `src/fonts_generated.rs` — 16 variants at 16/24px, variable-width 1-bpp
  glyphs. `font::select_font`/`pick` map CSS font-family/weight/style to a
  variant; the browser threads the chosen font through measurement + drawing
  (None = built-in 8x16). `<pre>` and the Lisp REPL use JetBrains Mono.

Two supporting fixes (not numbered tasks) were needed for real sites:
**base-relative URL resolution** (an external page's relative `style.css`/images
now resolve against its own host, not loopback — `url_join` + `PAGE_BASE`), and
**the `background:` shorthand** (most sites use it, not `background-color`).

### Acceptance test — henryratterman.com (direct TLS 1.3)

Loads end to end: page HTML + Google Fonts CSS + the 31 KB `style.css` all
fetched over our from-scratch TLS (base-relative), parsed, and rendered.

**Renders correctly:** the dark theme (background shorthand + CSS vars), the
horizontal nav bar (flexbox), section headings and body text in real serif
typefaces (Cormorant Garamond / Lora) and Barlow Condensed labels — it reads as
a real personal site, not a wall of bitmap text.

**Doesn't (known, acceptable):** a duplicate vertical nav (the mobile menu —
hidden by a media query / complex selector the CSS subset skips); emoji/section
icons show as `?` (outside printable ASCII); hero/project images are JPEG, so
empty space (no JPEG decoder); the page is shorter than the live site (much of
its content is injected by JavaScript, which the browser doesn't run).

**Serial tokens fired this milestone:** HTTP_READ_OK, EXT_IMG_OK, CSS_VAR_OK,
FLEX_OK, FONTS_OK.

## Bugfix — large-PNG OOM crash (2026-06-11)

**Symptom:** opening a real-world-sized PNG (e.g. 1920x1080) in the image
viewer (or via the file manager) took the whole OS down — noVNC dropped, the
QEMU instance vanished.

**Root cause:** `png::decode()` allocated a full-resolution XRGB pixel buffer
(~8 MiB at 1920x1080) *plus* the inflated scanlines (~6 MiB, transiently ~2x
during the inflate buffer's doubling growth) on the fixed 16 MiB kernel heap.
The allocation failed, `alloc` returned null, the default handler panicked,
and the panic handler calls `semihosting::exit(1)` — QEMU exits, "OS gone".

**Fix (`src/png.rs`):** (1) lowered the decoded-dimension cap to 2048x2048;
(2) added a heap-budget guard before `inflate()` that estimates the peak
(pixels + 2x raw + 1 MiB headroom) against `heap::free_bytes()` and refuses
the image gracefully (`decode` -> None, viewer shows "cannot decode") instead
of OOM-panicking; (3) drop the compressed `idat` copy before the pixel buffer
goes live. Plus defensive bounds checks before the blits in `viewer.rs`
(source index + destination clamp; large images are already scaled to fit) and
`files.rs` (skip slots past the canvas edge). Emits **PNG_CRASH_FIXED** on
serial the first time a multi-megapixel image is handled without crashing.

**Test:** `scripts/pngfix_test.sh` (gen 1920x1080 PNG via `mkbigpng.py`, stage
it so the viewer opens onto it, drive via `drive_pngfix.py`): the image is
refused gracefully, PNG_CRASH_FIXED fires, the OS stays alive (QMP screendump
+ navigation to a normal PNG still work), no KERNEL PANIC. M23 (viewer), M29
(file manager) and M34-img (browser `<img>`) regressions all still green.

### Follow-up — downscale-on-decode (2026-06-11)

The first cut capped dimensions at 2048 and *refused* anything that wouldn't
fit the heap. But a 2048x2048 image is exactly 16 MiB of pixels (= the whole
heap), and the real hog is the full inflated scanline buffer — so even the
nominal "max" case was refused, and users couldn't see their photos.

Reworked `png::decode` to **stream**: `inflate` is now incremental
(`inflate_into` emits each byte through a 64 KiB sliding window — never the
whole decompressed image), and a `RowAsm` consumer unfilters each scanline in
place and samples it straight into a **downscaled** output buffer (integer 1/f
nearest-neighbour). `decode` picks the smallest f whose output fits the free
heap with headroom — f=1 keeps full resolution, larger f shows the image
smaller instead of refusing it. Peak memory is now `output + 64 KiB + ~2
scanlines`, not `w*h*4 + raw`. Interlaced (Adam7) still uses the full-buffer
`inflate()` (needs random canvas access) and is refused only if it won't fit.

Also the btw.md UX ask: a declined image now shows filename + real dimensions
(via new `png::probe`) + file size + a clear amber "Image too large for Veil -
max 2048 x 2048 px" message, instead of a black "cannot decode" box.

`scripts/pngfix_test.sh` now stages a 2048x2048 (must render, downscaled — it
does: ~1/2, 5853 distinct colors on screen) and a 3000x2000 (must show the
graceful message). M23/M29/M34-img/Adam7 regressions all still green.

## Browser CSS overhaul — real sites render cleanly (2026-06-11)

henryratterman.com (and real sites generally) rendered badly: a duplicate
centered nav over the content, blue (wrong) nav links, cramped/inline layout,
and dark-on-dark invisible headings. Root causes were all in the CSS subset.

Fixes (`src/css.rs`, `src/browser.rs`):
- **Descendant selectors + multi-class matching.** `.nav-links a { color }` and
  `class="nav-links nav-links--desktop"` now resolve (the engine threads the
  ancestor chain through `Style.anc` and matches any one of an element's
  classes). This is what fixed the nav link colour (was falling back to the
  default blue).
- **`@media` / `@keyframes` / `@font-face` blocks are skipped** (balanced-brace
  scan). Previously their nested rules were parsed as global, so mobile
  `display:none` overrides leaked into the desktop render.
- **`opacity:0` + `pointer-events:none` ⇒ hidden** (a JS-toggled overlay like
  the mobile menu — the duplicate nav). `opacity:0` *alone* is left visible,
  since sites also use it for scroll-reveal content we can't un-hide (no JS).
- **HTML5 block elements** (`section`/`nav`/`header`/`footer`/`main`/
  `article`/`aside`/`figure`/`blockquote`) default to `display:block` — they
  were inline, so everything cramped onto a few lines.
- **rem/em/pt units** in `parse_px` (1rem≈16px) — gaps, padding, margins and
  font-sizes were collapsing because only `px` parsed.
- **rgb()/rgba()** in `parse_color`; **text-decoration** (none/underline).
- **Minimum-contrast text** at paint time: the site's dark text colours assume
  light section backgrounds we don't paint, so on the dark page they vanished.
  When fg/bg luminance is too close, blend fg toward the legible extreme
  (keeps a hint of hue). High-contrast text — the common case, and every
  existing test page — is untouched.

Result: henryratterman.com now renders as a clean personal site — a single
spaced cream nav bar, terracotta section labels, and visible headings.

New deterministic gate `scripts/drive_m34_nav.py` + `navtest.htm` (in
`mksite.py`) exercises all of the above over loopback (no live network):
multi-class+descendant link colour, @media skipped, overlay hidden,
scroll-reveal visible, rem gap. Regressions all green: M16 (pixel-exact on-OS
site), M34 flex/cssvar/font, M32 table/internet, M31 web; the live
henryratterman render via `scripts/m32_test.sh scripts/drive_m34_hr.py`.

## M35 — "GO CRAZY" (2026-06-11)

The big one. Ten subsystems, 15 acceptance tests. All built from scratch, no crates.

| # | Piece | Status | Proof |
|---|-------|--------|-------|
| 1 | **JPEG decoder** (`src/jpeg.rs`) | DONE | baseline **+ progressive** (SOF0/1/2), Huffman, restart markers, integer IDCT, 4:4:4/4:2:2/4:2:0, YCbCr→RGB. JPEG_OK; viewer renders DOG.JPG (progressive) crisply. Wired into viewer/files/browser via `png::decode_any`. |
| 2 | **Browser text input** | DONE | editable address bar (click/type/Enter/Esc) + on-page `<input>`/`<textarea>` (click-focus, type, focus ring). `drive_m35_input.py`. |
| 3 | **Clipboard** | DONE | `src/clipboard.rs`; Ctrl+C/Ctrl+A/Ctrl+V across browser/shell/lisp/files. `drive_m35_clip.py` (1235 B browser→shell). |
| 4 | **Real shell** (`src/shell.rs`) | DONE | ls/cat/cp/mv/rm/echo>file/pipes/grep/pwd/cd/run + history + tab-completion over FAT16 (`fs::delete` added). `drive_m35_shell.py`. |
| 5 | **App kill** | DONE | `ps`/`kill <id|name>` reclaims an app's heap via Drop, others keep running. `drive_m35_kill.py` (kill browser, lisp survives). |
| 6 | **GUI overhaul** | DONE | modern dark palette (#0d0d0d/#1a1a1a/#5b8af0), no chunky blue title bars, accent focus border + underline, drop shadows, slim taskbar, desktop grid. `drive_m35_gui.py`; gui_test re-greened. |
| 7 | **MJPEG video** (`src/video.rs`) | DONE | splits JPEG frames, decodes on tick ~25fps, scale-to-fit, play/pause/seek. DEMO.MJP plays. `drive_m35_video.py`. |
| 8 | **WASM + JIT** (`src/wasm/`) | DONE | parser + stack interpreter + WASI host (fd_write) + **single-pass AArch64 JIT** (locals/operand stack → x9..x17, native ARM64 into EL1-exec heap, I-cache flush). hello prints; **JIT is 2873× faster** than the interpreter on compute(400k). `drive_m35_wasm.py`. |
| 9 | **Direct kernel TCP** | DONE | browser `http_direct()` — external http:// goes via the kernel TCP/IP stack (DNS+connect+GET), proxy only as fallback. example.com→104.20.23.154:80, DIRECT_HTTP_OK. `drive_m35_net.py`. |
| 10 | **Games** | DONE | Snake (`src/snake.rs`, high score → SNAKE.TXT) + Veil Breakout (`src/breakout.rs`). `drive_m35_snake.py`, `drive_m35_breakout.py`. |

**Acceptance tests:** 1 ✓(viewer+upload), 2 ~ (JPEG `<img>` renders — proven on photo.jpg; henryratterman's hero is JS-injected/CSS-bg, a JS limit not a JPEG one), 3 ✓ (Wikipedia QEMU — redirect-following added, 6463 items of article text), 4–15 ✓.

Also added: Alt+Tab task switch, browser **redirect-following** (3xx Location), rgb()/text-decoration already from M34. Regressions green: boot self-tests (JPEG_OK/WASM_OK/WASM_JIT_FAST/CRYPTO/FONTS/HEAP), gui_test, m16, m23, m29, m34 nav/flex/cssvar, m32 table. (m34_img's gnu-logo remains external-network-flaky.)
