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

## M37 — from-scratch H.264 + MP3 decoders (2026-06-11)

Two more from-scratch codecs, no crates, wired into the apps + file manager.
Proof: `scripts/m37_test.sh` (boot self-tests + GUI drivers + headless sound).

- **H.264 baseline decoder** (`src/h264/`): constrained-baseline (I + P slices,
  CAVLC, single reference, 4:2:0). `bits.rs` (RBSP unescape + Exp-Golomb),
  `mod.rs` (Annex-B start codes **and** MP4/ISO-BMFF demux: moov/trak/stbl,
  stsz/stco/stsc sample table, avcC parameter sets, SPS/PPS Exp-Golomb),
  `cavlc.rs` + `cavlc_tables.rs` (coeff_token/level/total_zeros/run_before VLC,
  ITU Table 9-5/9-7/9-10), `transform.rs` (4×4 integer inverse transform,
  dequant, luma/chroma Hadamard DC), `slice.rs` (slice header, macroblock layer,
  intra 4×4 9-mode / 16×16 4-mode / chroma prediction, P-slice motion
  compensation with 6-tap quarter-pel luma + bilinear eighth-pel chroma, median
  mv prediction, in-loop **deblocking filter** §8.7, limited-range YCbCr→RGB).
  Validated against ffmpeg's decode of the same clip: quadrant colours within
  ±2, P-frame box position pixel-exact, deblocked boundary within ±1.
  **H264_OK.** Wired into the video player (`video.rs` pre-decodes `.mp4`) +
  file manager (`.MP4` → Video). `scripts/drive_m37_video.py`.

  *Three bugs found + fixed during bring-up:* (1) intra-4×4 **mode** prediction
  used the reconstruction grid (`decoded4`) for neighbour availability, but
  intra-MB neighbours aren't reconstructed yet during the mode-reading pass —
  added a separate `i4set` "mode known" grid (this was a +52 luma offset
  smeared across whole regions via horizontal/vertical intra propagation).
  (2) The `mb_skip_run` state machine read a *new* skip run after exhausting one,
  swallowing the coded MB that follows — so every P frame decoded as all-skip
  (motion never applied, the box never moved). (3) dequant overflowed i32 on a
  stray coefficient → switched to i64 with a clamp (defensive).

- **MP3 Layer III decoder** (`src/mp3/`): full pipeline ported from the
  public-domain pdmp3.c (Unlicense) to Rust over a whole in-memory buffer (no
  streaming ring): frame header/sync, side info, scalefactors, Huffman decode,
  requantization, reorder, M/S + intensity stereo, alias reduction, IMDCT +
  windowing (hybrid synthesis), frequency inversion, polyphase synthesis
  filterbank → 16-bit PCM. The kernel has **no libm**, so all fixed cos/sin
  windows and the `is^(4/3)` table are precomputed by `scripts/gen_mp3_tables.py`
  and requant exponents (always multiples of ¼) use f64 exponent-bit math.
  Decodes the embedded 440 Hz tone to a clean 440 Hz fundamental, peak 3953 vs
  ffmpeg's 4023. **MP3_OK.** Wired into `snd::play_file` (`.MP3` → decode →
  resample-to-44100 → virtio-sound) + the audio app + file manager.
  `scripts/drive_m37_mp3.py`; headless `opt/veil.mode=mp3` streams it through
  virtio-sound (**AUDIO_OK**).

**Table provenance:** all bulky/error-prone tables are *generated* from vendored
reference sources (committed under `vendor/`) so they're verifiable, not
hand-transcribed: MP3 Huffman/window from `pdmp3.c`; H.264 CAVLC VLCs from
FFmpeg `h264_cavlc.c`; deblock α/β/tc0 from FFmpeg `h264_loopfilter.c`. The
table *values* are the ITU-T H.264 / ISO 11172-3 spec tables.

**Serial tokens fired this milestone:** H264_OK, MP3_OK (+ AUDIO_OK via the MP3
sound path). Regressions green: gui_test, m29 (file manager), m35 MJPEG video,
and the JPEG/WASM/FreeType/crypto boot self-tests.

## Release kernel for hosted sessions (2026-06-11)

Hosted visitor sessions now boot the optimized **release** kernel (~2 s to the
desktop vs ~20 s debug; codecs decode an order of magnitude faster).
`session_manager.py` `KERNEL` → `target/aarch64-unknown-none/release/veil`.

The switch exposed a **release-only fw_cfg DMA bug**: the "QEMU" signature (and
every fw_cfg directory/file read) is filled by the device via DMA, which the
optimizer can't see — in release it assumed the caller's stack buffer was
unchanged, so `from_dtb` returned None → no framebuffer, no `veil.relay`/
`fastboot` flags. Fixed in `fwcfg::dma` with a `dsb sy` + `core::hint::black_box`
on the buffer pointer after each transfer so reads actually reload from memory.
Also added an `opt/veil.fastboot` flag (set per visitor spawn) that skips the
~16 s debug-build codec self-tests; harmless on release but keeps any future
debug deploy fast.

## M38 — browser overhaul: from-scratch JS engine + web fonts + CSS grid (2026-06-11)

Goal: make `https://henryratterman.com` actually render (it's JS-rendered — the
static HTML is an empty skeleton; `content.js` holds the data and an inline
`render()` builds the DOM).

- **From-scratch JavaScript engine** (`src/js/`): lexer (`lexer.rs`, incl.
  template literals with `${}` interpolation), recursive-descent parser
  (`parser.rs` — full operator precedence, arrow functions, destructuring,
  for/of, try/catch, ternary, spread), tree-walking interpreter (`interp.rs` —
  scopes/closures, the Array/String/Object/Math methods, `this` binding) and a
  DOM binding layer (`dom.rs` — an index-addressable arena: getElementById,
  querySelector(All), createElement/appendChild, innerHTML/textContent setters
  that re-parse, classList, style, dataset; document/window/console/localStorage/
  matchMedia/history/location host objects; setTimeout/requestAnimationFrame
  deferred + drained). The kernel has no libm, so `mathf.rs` does floor/ceil/
  trunc/sqrt via AArch64 `frintm/frintp/frintz/fsqrt`. Runs the **real**
  shared.js + content.js + inline render() unmodified: **JS_OK** boot self-test
  injects "Henry Ratterman", the headshot src, and 12 project cards.
- **Browser integration** (`browser.rs`): `collect_scripts` gathers inline +
  same-origin `<script>` (skips cross-origin analytics), runs them via `js::run`
  against the parsed tree, and lays out the mutated DOM. Required making
  `<script>`/`<style>`/`<textarea>` **raw-text elements** in `html.rs` (their
  content has `<`, `>`, template literals that must not be parsed as markup).
- **Web fonts** (`freetype.rs` + `browser.rs`): btw.md said "WOFF2 = gzip TTF"
  but WOFF2 is **Brotli + transformed tables** (a huge decoder) — instead, Google
  Fonts serves **plain TTF** to our generic `VeilOS` User-Agent, so we just fetch
  the TTF and feed FreeType (which already loads TTF). `register_font_faces`
  parses `@font-face` rules from the fetched stylesheets, fetches the TTFs
  (bounded; magic-checked; `.ttf`/`.otf` only so a self-hosted woff2 @font-face
  doesn't claim a family's slot), and registers dynamic `FontId::Web(i)` faces.
  `pick_ftid` prefers a registered web font (Cormorant Garamond / Barlow
  Condensed / Lora) over the bundled fallback.
- **CSS grid** (`browser.rs`): `display:grid` + `grid-template-columns`
  (counts tracks, expands `repeat(N,…)`) + `gap`. `layout_grid` flows items
  row-major into equal columns. **GRID_OK**.
- **DOM-injected images** already work: the JS sets `img.src=headshot.jpg`, the
  layout fetches + decodes it (JPEG via `png::decode_any`).

Loopback proof `scripts/drive_m38_js.py` (JSTEST.HTM = the real scripts inlined):
body text grows from ~empty to 36 KB, 933 layout items, 411 colours. Live
acceptance `scripts/drive_m38_hr.py` over `scripts/m38_test.sh` (release) renders
henryratterman.com over direct TLS: HTML + shared.js + content.js + style.css +
6 web-font TTFs + headshot.jpg all fetched, scripts run (21 KB of injected text),
web fonts registered, grid laid out, 255 colours. **Tokens:** JS_OK, GRID_OK.

Two hangs found + fixed during bring-up: (1) web-font glyphs rendered with
`FT_LOAD_TARGET_LIGHT` ran the autofitter, pathologically slow on Cormorant —
switched web fonts to `FT_LOAD_NO_HINTING` (smooth rasteriser still
anti-aliases). (2) the multi-MB page buffer OOM-**panicked** the browser on a
10000+px JS-rendered page over a fragmented 16 MB heap — now free the old page
first and allocate with `try_reserve`, shrinking `doc_h` until a contiguous
block fits (fill_rect/blit clip).

## M39 — browser UX: tabs, back/forward, zoom (2026-06-11)

- **Tabs** (`browser.rs`): `BrowserState` gained `tabs: Vec<Tab>` + `active`.
  Each `Tab` carries its own path/scroll/back/forward/title; the active tab's
  rendered page lives in the existing fields and switching re-renders (no 5 MB
  page buffer per tab). A tab strip (22 px) sits above the address bar — click a
  tab to switch, its `x` to close, `+` for a new tab. Ctrl+T new tab, Ctrl+W
  closes the active tab (not the window). `tab_layout` shares geometry between
  paint and click routing.
- **Back/forward**: per-tab back + forward stacks; a navigation pushes the old
  path to back and clears forward; the address-bar row gained a `>` forward
  button beside `<`. HISTORY_OK.
- **Zoom**: Ctrl+= / Ctrl+- / Ctrl+0 adjust a per-tab zoom (50–250 %) threaded
  into layout's font-size computation via a `ZOOM` static; re-renders the page.
- **Ctrl+click a link opens it in a new tab** (the WM passes its Ctrl state into
  the browser content-click dispatch).
- Chrome height went from `TOPBAR` (20) to `CHROME = TABBAR_H + TOPBAR` (42);
  all page-content offsets + the browser proof drivers updated by +22 px.

Proof `scripts/drive_m39_tabs.py` (loopback): Ctrl+click opens a new tab, switch
between tabs, navigate + back + forward in a tab, zoom in/reset — ALL CHECKS
PASSED. Regressions (m16, m34, m38) updated for the tab bar and green.

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

## M40 — Browser + OS capability push (2026-06-11, in progress)

Driven by `.claude/btw.md` (8 steps, deploy after each).

| # | Step | Status | Proof |
|---|------|--------|-------|
| 1 | **Form POST + cookies** | DONE | `<form method=POST>` submits `application/x-www-form-urlencoded`; cookie jar (`COOKIE_JAR`, parse `Set-Cookie`, per-domain `Cookie` header); follows 302/303 after POST. Kernel server gained `/login` (302 + Set-Cookie), `/welcome` (cookie-aware), `/echo` (query echo). `drive_m40_forms.py`: login POST → redirect → cookie sent → server logs LOGGED IN. |
| 2 | **Input elements** | DONE | text/password/hidden/checkbox/radio/select/textarea/submit all render + submit. Checkbox toggles, radio is one-per-name-group, `<select>` cycles options and submits the `value` attr, GET form appends query string. Live state repainted via `paint_fields` (no re-fetch). `drive_m40_forms.py` submits all kinds: `t=hi&c1=yes&color=red&size=M&msg=note`. (Label-click focus is a remaining minor gap.) |

Fixes along the way: redirect from a loopback page no longer fabricates `https:///path` (uses `resolve_href` for local); `resolve_href` keeps the query string for local paths (GET forms need it).
| 3 | **File downloads** | DONE | Non-renderable response (anything not `text/html`) is saved to the FAT16 disk with an 8.3 name derived from the URL/content-type, and a "Downloaded NAME" toast shows (new global `wm::queue_toast` drained each `clock_tick`, so the browser can toast from a `&mut Window`). Kernel server learned PDF/ZIP/MP4/JSON mime types. `drive_m40_download.py`: click a `/sample.pdf` link → SAMPLE.PDF saved → toast → listed in Files. |
| 4 | **OS-wide scrolling** | DONE | Mouse-wheel now routes to the focused window's app, not just the browser. Editor gained a `scroll` field (follows the cursor until you wheel up); shell gained scrollback (`scroll` on the Shell variant); files (`files::wheel`) and the Lisp REPL (`repl::wheel`) scroll too. Positive notch = up; typing/new output re-pins to the bottom. `drive_m40_scroll.py`: a 250-line BIG.TXT in the editor and `cat big.txt` in the shell both wheel up to line 1 (top=0). |
| 5 | **henryratterman.com full navigation** | DONE | Root cause of the clipping found+fixed: the browser rasterized the *entire* document into one buffer (480×10546×4 ≈ 20MB), which the 16MB heap couldn't hold, so it shrank to ~1333px and clipped everything below the hero. Rewrote the renderer to keep a **retained display list** (`items`+`imgs`) and rasterize only a moving **band** (≤2000px) repainted on scroll — the full logical height (10546px) is now scrollable. `drive_m40_hrnav.py`: page lays out full height, PageDown re-rasterizes the band to top=8546, the footer/project links are present, and the bottom paints content. m16 regression green. |
| 6 | **Reddit + Twitter smoke test** | DONE | reddit.com / www.reddit.com are a blank React SPA without their JS bundle, so the browser now transparently routes them to **old.reddit.com** (server-rendered) — a real front page renders (375 items / 161 links). x.com serves its server-side "JavaScript is not available" page, which the browser renders cleanly (57 items, the real X fallback). Also fixed HTML entity decoding (`&raquo;`→», `&mdash;`, `&hellip;`, `&copy;`, smart quotes, arrows, hex `&#xNN;`, …) — `more »` now reads correctly across all sites. `drive_m40_sites.py`. |
| 7 | **Viewport-aware image loading** | DONE | Images are now an `ImgSlot` table sized from width/height attrs (or a default box). Render does a 2-pass layout: pass 1 finds each image's y with placeholder sizes, then only images within ~2 viewports of the top are fetched+decoded; pass 2 re-lays-out with the decoded sizes. Off-viewport images stay placeholders and are fetched lazily on scroll (`lazy_load_images`, called from `scroll_to`), then nearest-neighbour scaled into their box (`fb::blit_scaled`) — no re-layout. Bounds memory+network on image-heavy pages. `drive_m40_lazyimg.py`: 10-image page decodes 3 at load, all 10 after scrolling; m16 (logo 64x64) + m38_hr green. |
| 8 | **`<video>` tag in the browser** | DONE | The browser parses `<video src>` / `<video><source src>`, renders a dark poster box with a play triangle + "click to play video", and registers a hit box. Clicking fetches the MP4 (`http_request`) and hands it to the WM (`wm::queue_video`, drained each tick) which opens the existing video player via the new `VideoState::with_data` — the H.264 pipeline decodes + plays, with the player's play/pause (Space) + seek controls. Direct MP4 only. `drive_m40_video.py`: a `<video src="quad.mp4">` page → click → QUAD.MP4 decodes to H.264 and plays. |

All 8 steps PASSED + deployed. Net new browser capabilities: form POST + cookie jar, full input controls, downloads, OS-wide wheel scroll, band rasterizer (full-height JS pages scroll), reddit/X compat + HTML entities, lazy viewport image loading, and inline `<video>` playback.

## M41 — "The Real Push" (2026-06-12, 21-step brief, in progress)

Driven by `.claude/btw.md` (PHASE 1 browser, 2 shell, 3 app platform, 4 security, 5 ambitious). Deploy + PROGRESS update after each step.

| # | Step | Status | Proof |
|---|------|--------|-------|
| 1 | **ES6+ JavaScript engine** | DONE | Upgraded the from-scratch JS engine (src/js/) from ES5 to ES6+. Added: **classes** (constructor, methods, `extends`/`super()`/`super.m()`, static, fields, instanceof), **object destructuring** + **default params** + **object spread**, **async/await** + **Promise** (synchronous-resolution model: resolve/reject closures, then/catch/finally, Promise.resolve/reject/all/allSettled/race), **fetch()** wired to the real HTTP/TLS stack (`browser::js_fetch` → Response with `.text()`/`.json()`/`.ok`/`.status`), **Map/Set/WeakMap/WeakSet** (get/set/has/delete/forEach/keys/values/entries/size), **Object.keys/values/entries/assign/freeze/create/fromEntries/defineProperty**, **Array.from/isArray/of**, **Number.isInteger/isFinite/isNaN**, **String.fromCharCode**, **JSON.parse/stringify** (from-scratch), **Symbol** (stub), getters/setters, generators (`function*`/`yield` parsed, degenerate), **ES module import/export** (parsed; one shared global), nullish `??` + optional chaining `?.` (already present), template literals/arrows/for-of (already present). Proof: boot self-test `JS_ES6_OK` (12 features verified incl. `super.describe()` + `instanceof`); `drive_m41_fetch.py` proves `await fetch('/echo')` + `await res.text()` round-trips over the HTTP stack (status=200, body received). m38_js (henryratterman render, 36k DOM chars) + JS_OK (12 project cards) green. NOTE: react.dev (multi-MB minified bundle) is aspirational — the engine executes modern async/fetch apps but a full React runtime is beyond a tree-walker over our network. |
| 2 | **JavaScript JIT compiler** | DONE | Single-pass AST→AArch64 JIT (`src/js/jit.rs`) for the numeric, call-free subset of JS functions (arithmetic, %, comparisons, &&/\|\|, if/while/for, locals, return, ternary, ++/--). Every value is an f64 in an FP register (d0..d7); params arrive via a pointer to an f64 array; code is written into a `Vec<u32>` in EL1-executable kernel RAM, caches flushed, called through a fn pointer — same approach as the WASM JIT. **Profile-guided + deopt**: the interpreter keeps a per-function cache (`try_jit`), compiles after a call threshold, and runs native only when all args are numbers — anything else (non-numeric args, member access, calls, closures, strings) deopts to the tree-walker. Fixed two codegen bugs: SCVTF int→double encoding (0x1E62 not 0x1E22), and an **ABA hazard** where a freed arrow's heap address was reused by the next arrow inheriting stale compiled code (now the cache pins the `Rc<Func>`). Proof: boot self-test `JS_JIT_FAST` — `bench(300000)` (a `%`/`*`/`/`/compare/if hot loop) gives bit-identical results and runs **~450x faster** than the interpreter (pass bar 50x). ES6 suite + henryratterman render (m38_js, 36k DOM chars) green with JIT auto-enabled. |
| 3 | **WebSockets** | DONE | From-scratch RFC 6455 client (`src/websocket.rs`): from-scratch **SHA-1** + **base64**, the HTTP **Upgrade handshake** with `Sec-WebSocket-Accept` verification, masked client→server / unmasked server→client **framing** (text/binary/ping→auto-pong/close, 7/16/64-bit lengths, fragmentation reassembly). Runs over a `Stream` trait: `ws://` on a plain TCP `net::Handle`, `wss://` on a `tls::TlsConn` (reuses the from-scratch TLS 1.3). JS binding: `new WebSocket(url)` + `send`/`onopen`/`onmessage`/`onclose`/`onerror`/`readyState`/`close`/`addEventListener`, dispatched through the engine's deferred (event-loop) queue (`browser::js_ws_open`/`js_ws_send_recv`/`js_ws_close` registry). Kernel HTTP server gained a loopback `/ws` **echo endpoint** (any `Upgrade: websocket` request). Proof: boot self-test `WS_PROTO_OK` (SHA-1/base64/accept match the RFC 6455 example vectors); `drive_m41_ws.py` — a page opens `ws://veil/ws`, `onopen→send('hello veil')`, the server echoes, `onmessage` receives it (`readyState=1`), then `close()`. Forms/HTTP serving regression green (the WS branch is a no-op for normal requests). |
| 4 | **LocalStorage + SessionStorage** | DONE | Real Web Storage API, keyed by **origin** (page host, "veil" for the local site), surviving across the per-render JS interpreter instances via browser-side statics. `localStorage` **persists to FAT16** (`LOCALSTG.DAT`, `origin\tkey\tvalue` lines, written on every set/remove/clear); `sessionStorage` is in-memory (separate store). Full API: `getItem`/`setItem`/`removeItem`/`clear`/`key`/`length` + direct property get/set (`localStorage.foo = x` ≡ `setItem`). New `Host::SessionStorage`; `browser::storage_*` hooks. `drive_m41_storage.py`: a page increments a localStorage + a sessionStorage counter per load; after navigating away and back (fresh interpreter) both read back incremented (lc/sc 1→2), `length`=2, direct-property `note` persisted. |
| 5 | **Browser text selection + copy/paste** | DONE | Each laid-out word becomes a `SelRun` (original-case text + document-coord geometry + font, reading order); a selection is a (run, char) **anchor + cursor**. Mouse-down on empty page area (after link/field/video hit-tests miss) starts a drag (`sel_begin`), mouse-move extends it (`sel_extend`, routed via the WM's `forward_mouse_move` while `is_selecting`), mouse-up commits (`sel_end`). `sel_hit` snaps a point to the nearest run (vertical distance dominates, x is the within-line tiebreak) and `sel_char_at` does midpoint-rounded per-glyph hit-testing for char offset. Painted as a **blue wash** (`blend_rect`) over the selected portion of each visible run in `paint_view`. **Ctrl+A** selects every run (highlight, pixel-independent); **Ctrl+C** copies the selection if present else the whole page (`selected_string` joins words with spaces, rows with newlines); **right-click menu** gained Copy / Select All (`MenuTarget::Browser`); **middle-click** pastes the clipboard into the text input under the cursor (new `BTN_MIDDLE` decode + `on_middle_down`, focuses the field first). The **editor gained a paste handler** (`editor_paste`, appends at the end-of-buffer cursor) — it had none. Proof `scripts/drive_m41_select.py` (over `m32_test.sh`, needs a NIC): drag-selects 163 B of the homepage → Ctrl+C copies the selection → Ctrl+A + Ctrl+C copies the full 1234 B page → editor Ctrl+V pastes it in (status shows "1234 bytes"). Screenshot `shots/m41_select_browser.png` shows the whole page highlighted blue. |
| 6 | **CSS — close the gaps** | DONE | The browser is a **static one-pass layout** engine, so the feasible high-value gaps were closed and the interactive ones documented. **`calc()` / `clamp()` / `min()` / `max()`**: `parse_px` now delegates to a recursive `eval_len` — `fn_args`/`split_top` peel the math functions, a `calc()` recursive-descent evaluator (`calc_tokens`/`calc_expr`/`calc_term`/`calc_factor`, * / before + -, parens, signed terms) folds resolvable length terms to px; viewport/percent units (%/vw/vh) are treated as unresolvable (dropped from min/max, fail a calc), `clamp(a,b,c)` returns `b` clamped to [a,c] or the a/c mean when the preferred (vw) term is unknown. **`@media (prefers-color-scheme: dark)`** is now **applied** instead of skipped (`css::is_dark_media` + recursive `parse` of the block) since Veil renders dark — `light` and `max-width` (mobile) queries still skipped. **`border-radius`** on a block background: new `Style.radius`, `Item::RoundRect` painted via `fill_round_rect` (falls back to a square fill only when the box straddles a band edge). Serial tokens `CSS_CALC_OK` / `CSS_DARK_OK` / `CSS_RADIUS_OK`. Proof `scripts/drive_m41_css.py` (loopback `csstest.htm` in `mksite.py`): calc/clamp/min/max evaluate, the dark-scheme `.box` bg + `.darkonly` green text apply while the light query is skipped (pixel-checked green not red), the rounded card renders. `shots/m41_css.png` shows the rounded green dark-mode card + clamp()-sized heading. **Not feasible in a no-live-event-loop renderer (documented):** `:hover`/`:focus`/`:active` recompute, `position: sticky`/`fixed` scroll-linking, `overflow: scroll/auto` independent scroll contexts, CSS transitions/`@keyframes` animation, `transform: scale/rotate`, true partial `opacity` compositing, and `z-index` stacking — these need continuous CSS-driven repaint the static layout pass doesn't have. Regressions: m34_nav (CSS engine), gui_test green. |
| 7 | **Canvas API** | DONE | A from-scratch HTML5 `<canvas>` 2D rendering context (`src/js/canvas.rs`): an ARGB pixel buffer + the full context state (fillStyle/strokeStyle/lineWidth/globalAlpha/font, a **2×3 affine transform**, save/restore stack, current path). Rasterizer: `fillRect`/`strokeRect`/`clearRect`, path building (`beginPath`/`moveTo`/`lineTo`/`rect`/`arc`/`ellipse`/`bezierCurveTo`/`quadraticCurveTo`/`closePath`), **scanline polygon `fill()`** (even-odd) + **thick-line `stroke()`** (rotated-quad per segment), `fillText`/`strokeText`/`measureText` (FreeType into the buffer), `drawImage` (canvas→canvas), `getImageData`/`putImageData` (RGBA byte arrays), `save`/`restore`/`translate`/`scale`/`rotate`/`transform`/`setTransform`. CSS color parsing incl. `rgb/rgba/hsl/hsla/#hex/named`, source-over alpha blending, kernel-`mathf` sin/cos/sqrt (no libm). **JS bindings**: `getContext('2d')` on a `<canvas>` node allocates a `Canvas` (sized from width/height attrs), stamps a `__cvs=N` attr on the element, returns `Host::Canvas(N)`; context property sets/gets + method dispatch (`canvas_method`) wired into the interpreter. **Browser integration**: after scripts run, each drawn buffer is flattened over white into XRGB and registered as a `__canvas:N` image slot, so the existing img layout/paint path blits it where the `<canvas>` sits. Proof: boot self-test **CANVAS_OK** (`js::canvas_selftest` — fillRect/arc-fill/stroked-line/fillText pixels verified) + `scripts/drive_m41_canvas.py` (loopback `canvas.htm`): a 6-bar bar chart with value labels + axis + title draws via page JS — `CANVAS_PAGE_OK`, 6 distinct bar colors pixel-confirmed. `shots/m41_canvas.png` shows the chart. **Limitation (documented):** a `requestAnimationFrame` game loop renders the final drained frame (a snapshot), not live animation — the browser runs a page's JS once at load, with no per-tick re-execution. JS engine regressions green: JS_OK, JS_ES6_OK, JS_JIT_FAST. |
| 8 | **IndexedDB (basic)** | DONE | A from-scratch **IndexedDB polyfill** (`assets/js/indexeddb.js`, `include_str!` as `js::INDEXEDDB_POLYFILL`) implementing the real async API — `indexedDB.open(name, version)` → request firing `onupgradeneeded`/`onsuccess` via `setTimeout` (drained through the engine's deferred queue, like a real event loop); `IDBDatabase.createObjectStore`/`transaction`/`close`; `IDBTransaction.objectStore`/`oncomplete`; `IDBObjectStore.put`/`add`/`get`/`getAll`/`getAllKeys`/`delete`/`clear`/`count` each returning an `IDBRequest` with `onsuccess`+`result`. **keyPath** auto-keying + **versioned upgrades**. Backed by the engine's **localStorage** (per-store `__idb:<db>:<store>` records, JSON-serialized via the from-scratch `JSON.stringify/parse`), so data **persists per-origin to FAT16** through the M41-step-4 path. Injected ahead of page scripts whenever a page references `indexedDB`; `window.indexedDB` resolves to it too. Proof: boot self-test **IDB_OK** (`js::indexeddb_selftest` — open→createObjectStore(keyPath)→put two structured records {id,title,tags:[…]}→get→getAll, round-trips `got=world tags=c count=2 first=hello`) + `scripts/drive_m41_idb.py` (loopback `idbtest.htm`): a visit counter in IndexedDB reads back **incremented across navigate-away-and-reload** (`IDB_PAGE visits=1`→`2`, fresh interpreter each load), proving persistence. JS regressions green: JS_OK, JS_ES6_OK, JS_JIT_FAST, CANVAS_OK. |
| 9 | **Real shell (bash subset)** | DONE | `src/shell.rs` rewritten as a **tree-walking interpreter**: tokenizer (quotes, `$(…)`/`${…}`/`$((…))` balanced, operators) → recursive-descent parser (AST: Simple/Pipeline/AndOr/List/If/For/While/Until/Case/FuncDef/Group) → executor. **Expansion**: `$VAR`/`${VAR}`/`${VAR:-def}`/`${VAR:=def}`/`${#VAR}`, `$?`/`$#`/`$@`/`$1..`, command substitution `$(…)` + backticks (captured stdout), arithmetic `$((…))` + `let` (full precedence evaluator: `+ - * / %`, comparisons, `&& || !`, parens, vars), single/double quoting + escapes, `~`, **word splitting** (quote-aware) and **glob** `*?[a-z]` against the FAT16 root. **Control flow**: `if/elif/else/fi`, `for x in …; do … done`, `while`/`until`, `case…esac` (glob patterns), functions (`name(){…}` / `function name`) with positional `$1..`. **Pipes** (each stage's stdout → next's stdin), **redirections** `>`/`>>`/`<`/`2>`/`2>&1`, `&&`/`||`/`;`. **Builtins**: `cd pwd echo(-n/-e) printf export unset set shift read let test/[/[[ type which source/. exit true false : env help run sh/bash` + `seq basename touch uniq`; the M35 leaf file/text commands (`ls cat cp mv rm grep(-i/-v/-n/-c) head tail sort(-r/-n) wc find date df`) preserved. State (vars/funcs/`$?`/positional) persists across REPL lines via a static. Proof: boot self-test **SHELL_OK** (`shell::selftest` — `arith=16 acc=123 cls=big n=3 tag=BIG hi veil sub=nested`, i.e. arith/for/if/while/case/function/cmd-subst+pipe) + `scripts/drive_m41_shell.py`: the GUI shell runs the seeded `TEST.SH` (build 3 files → for-loop iterate → pipe through `sort -r` → `> out.txt` → `wc -l` → `if`), giving `files=3`/`cherry`/`banana`/`apple`/`lines=3`/`RESULT_OK`, then interactive `echo $((6*7))`→42 and a `ls|grep|wc -l` pipe chain. **Cooperative limits (documented):** background `&`, `jobs`/`fg`, and Ctrl+C/Ctrl+Z signals are stubs — leaf commands run synchronously in the desktop task (no preemptive userspace process model for in-kernel shell builtins). |
| 10 | **Standalone binaries (coreutils + curl)** | DONE | The shell gained real text tools, implemented as builtins (the "binaries" — `which`/`type` report them as `/bin/<name>`; FAT16 is root-only so there's no on-disk `/bin` dir, documented). A from-scratch **regex engine** (`. ^ $ * + ? [a-z] \x`, anchors, backtracking) powers **grep** (now regex, `-i/-v/-n/-c/-l/-r`). **sed** (`s/old/new/[g]`, `/pat/d`), **awk** (`-F`, `/regex/` + `NR==k` patterns, `{print $1,$NF,NR}`), **cut** (`-d`/`-f`/`-c` with ranges `1,3`/`2-`), **tr** (set→set with `a-z` ranges, `-d`). **curl** (`curl [-s] [-o file] [-d data] URL`) fetches `https://` / external `http://` / local `/path` over the kernel HTTP/TLS stack (new `browser::shell_fetch`), body to stdout or a file. `mkdir -p`/`chmod`/`touch` accepted. Key fix: awk/sed/cut/tr take the real **argv** (not a re-joined string) so a quoted program like `{print $2}` keeps its spaces. Proof: boot **COREUTILS_OK** (`shell::coreutils_selftest` — `tr`→APPLE, `cut -d:`→b, `awk {print $2}`→10, `sed s//g`→barbar, `grep [0-9][0-9]`/`ND$`, and `awk -F, '/^p/{print $2}' | sort -r | head -1`→3, all through pipes+`$()`) + `scripts/drive_m41_coreutils.py`: the **acceptance** `curl https://henryratterman.com | grep Henry` returns "Henry Ratterman" (18 KB over direct TLS), `ls | grep .RS$ | wc -l` pipe chain, `cat demo.rs | sed s/fn/FUNC/g`, and `curl /index.htm | grep Veil`→`<h1>Veil OS</h1>` (loopback). |
