# Veil OS — progress

Contract: `os-build-spec.md` (M1–M17) + `os-build-spec-v2.md` (M18–M24).
Gated milestones; each passes only on observed proof.

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
