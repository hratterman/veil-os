# Veil OS — progress

Contract: `os-build-spec.md` (M1–M17) + `os-build-spec-v2.md` (M18–M22).
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
| M20 | Two-instance LAN chat | PASSED 2026-06-10 | `scripts/m20_test.sh`; confirmed via instance serial logs (A sent, B received + replied, both `CHAT_OK`) |
| M21 | GitHub release + hosted demo | not started | |
| M22 | Paint-save verify + polish | not started | |

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

## Post-M20 UX overhaul (no milestone — 2026-06-10)

Boot shows the bare desktop; nothing opens automatically. Apps launch
from a 40px bottom taskbar (Editor / Clock / Browser / Paint / Shell /
Chat-when-NIC) or the top-left desktop icon grid; both open-or-raise.
Every title bar has an 18px close (X) zone at its right edge. Windows
clamp above the taskbar. Default open positions unchanged.

**Known follow-up (deliberate, per instruction "build only"):** the
M6–M19 proof drivers assume windows exist at boot — they need a
launch-via-taskbar step before driving. Also the taskbar clamp shifts
shell/paint up 8px from their requested y (frames previously ended at
736 > 728), and drive_gui's beta drag to y≈640 will clamp. Re-verify the
suite when the drivers are updated (M22 polish pass is the natural slot).

## Kernel bugs found by milestone gates

- **M19 → timer drift:** `on_tick` re-armed with `TVAL = reload`, so IRQ
  latency stretched every period and `ticks()` fell behind wall time.
  Fixed: absolute `CVAL` deadlines, missed periods counted.
- **M17 → frames underflow:** an empty reserved range `(0,0)` wrapped
  `end - 1` and marked all frames used. Fixed: skip empty, tolerate
  overlapping ranges.
