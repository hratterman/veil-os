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
