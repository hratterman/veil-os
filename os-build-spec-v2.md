# Build Spec Extension: Veil OS v2 — M18–M22

**Prerequisite:** M1–M17 all pass. This document picks up immediately after M17.
**Same rules:** reality is the grader, gated milestones, no milestone accepted without observed proof criterion.

---

## M18 — Text Editor with Persistent Files

**What it is:** A new WM app (`App::Editor`) that opens a named `.TXT` file from the FAT16 disk, lets the user type and edit it, and saves it back on SAV button click (matching the paint save pattern).

**Scope:**
- New window: "edit" title bar, white background, monospace text
- Keyboard input via existing virtio keyboard driver
- Cursor (blinking or static, drawn as an inverted glyph)
- Backspace, Enter (newlines), printable ASCII
- Opens `NOTE.TXT` by default (creates it if missing)
- SAV button writes current buffer back to FAT16 via `fs::write_file`
- LOD button re-reads from disk, discarding unsaved changes
- No font scaling required — existing 8x16 bitmap font is fine
- No scrollback required for v1 — wrap lines at window edge, clip at bottom

**Pass criterion:**
```
scripts/drive_m18.py
```
Script types a known string into the editor via QMP keyboard injection, clicks SAV, reboots, clicks LOD, pixel-verifies the text reappears on screen. Serial must emit `EDITOR_OK`.

---

## M19 — Clock App (analog + digital, multiple faces)

**What it is:** A draggable WM window showing a real ticking clock. Time starts at 00:00:00 on boot and counts up from the existing 100ms timer. The window has a face-picker that cycles through styles with a click.

**Clock faces (cycle with a click anywhere on the face):**
- **Wall clock** — classic round analog face, Roman or Arabic numerals, hour/minute/second hands, tick marks. The showpiece.
- **Digital** — large HH:MM:SS in the existing bitmap font, clean, minimal.
- **Chronograph** — analog main dial (minutes elapsed) + two sub-dials (seconds, hours). Start/stop/reset buttons below.
- **Stopwatch** — digital centiseconds display (MM:SS.cc), start/stop/reset buttons.

**Rendering requirements:**
- Analog faces use Bresenham line drawing (already in the fb lib) for hands and tick marks
- Circle outline for the face drawn with the existing line primitives or a new `draw_circle` helper
- Hour and minute hands are thick (3px), second hand is thin (1px), different colors
- Smooth second-hand sweep (updates every 100ms tick, not just every second)
- Face fills the window content area, scales to window size

**Scope constraints:**
- Chronograph and stopwatch share the same timer accumulator, independent lap state
- No alarm functionality

**NTP time sync (M19b fix pass):**
- On boot, send a single NTP UDP request to `pool.ntp.org` (port 123), parse the 64-byte response, set the kernel wall-clock to real UTC time
- Read `TZ.TXT` from FAT16 on boot for UTC offset integer (e.g. `-5` for EST, `-4` for EDT). If missing, default to UTC.
- Clock app displays real local time, not "since boot"
- "since boot" label removed from wall clock face -- replaced with timezone label (e.g. `UTC-5`)
- If NTP fails (no network), fall back gracefully to time-since-boot with a small `(no sync)` indicator
- Fix wall clock layout: "since boot" text must not overlap the clock face

**Pass criterion:**
Serial emits `CLOCK_OK`. Proof script takes screenshots at t=0, t=1s, t=2s and pixel-verifies that the second hand pixel position changed each time across all four faces. NTP sync verified by serial line `NTP: set clock to <timestamp>`.

---

## M20 — Global Chat (the blow-away feature)

**What it is:** Every running Veil instance -- local or hosted -- joins a single public chat room over real UDP. You type in your local instance, it appears on the hosted server instance (and vice versa), visible to anyone watching the live demo at the public URL.

**Why it blows someone away:** Multiple completely independent operating systems, each written from scratch -- their own kernels, their own UDP implementations -- exchanging messages with each other over the real internet. Neither one is Linux. Neither one borrowed a network library.

**Architecture:**
- The M21 VPS runs a Veil instance that acts as the relay: it rebroadcasts every incoming message to all currently connected peers (tracked by source IP/port, with a 60s idle timeout)
- Local instances connect by pointing UDP at the server's public IP, port 7777
- `scripts/demo.sh` launches with the server IP pre-configured so it just works out of the box

**Scope:**
- New `App::Chat` window: scrollable message log + single-line input field
- On first launch, prompt for a username (max 12 chars), saved to `USER.TXT` on FAT16
- Messages sent as `username: text\n`, max 140 bytes total
- UDP send on Enter keypress → server IP:7777
- UDP receive loop (polled in scheduler) → append to log, scroll to bottom
- Server relay logic runs as a kernel task on the hosted instance, no separate process
- Local demo mode: if no server reachable within 3s, fall back to loopback between two local instances (for offline use)

**Pass criterion:**
`scripts/drive_m20.py` launches two local instances in fallback mode, injects a message into instance A, pixel-verifies it appears in instance B. Serial from both emits `CHAT_OK`. Manual proof: local instance sends a message that appears on the hosted VPS instance visible at the public URL.

---

## M21 — GitHub Release + One-Liner + noVNC Hosted Demo

**What it is:** The project becomes publicly runnable in two ways: (1) local one-liner, (2) hosted live demo anyone can open in a browser.

**Scope — GitHub:**
- Clean `README.md`: what it is, screenshot, milestone list with brief descriptions, one-liner
- One-liner (macOS):
  ```bash
  brew install qemu && git clone https://github.com/henryratterman/veil-os && cd veil-os && scripts/demo.sh
  ```
- `scripts/demo.sh`: calls `mkdisk.sh`, launches QEMU with correct flags (the cocoa display command we verified works), prints "Veil OS is running" to terminal
- `.gitignore`: excludes `target/`, `disk.img`, `shots/`, `*.ppm`
- MIT license

**Scope — Hosted demo:**
- VPS (cheapest DO droplet, $6/mo) running:
  - QEMU headless with `-vnc :0`
  - noVNC (websockify) proxying port 6080 → VNC
  - nginx serving noVNC static files + proxying `/websockify`
  - systemd unit that auto-restarts QEMU if it crashes
- Single public URL: `https://veil.henryratterman.com` (or subdomain of your choice)
- Clicking the URL opens the live Veil OS desktop in the browser, mouse and keyboard work
- QEMU instance resets every 30 minutes via a cron job (keeps it clean for demos)

**Pass criterion:**
`curl -I https://veil.henryratterman.com` returns 200. Manual browser test: open URL, click in the desktop, paint something. README one-liner runs clean on a fresh macOS machine (verified in a new terminal with no prior veil-os deps beyond Xcode CLT).

---

## M22 — Paint Save verified + polish pass

**What it is:** Confirm paint SAV/LOD actually survives reboot (the code exists but was never interactively verified), fix any rough edges found during M18–M21, update the hosted site content to reflect the full feature set.

**Scope:**
- Run `drive_m10_save.py` / `drive_m10_load.py` pattern against the interactive paint save path
- Update `scripts/mksite.py` to describe all features through M21
- Verify the browser window on the hosted site loads the updated site content
- Fix any pixel regressions in the full milestone regression suite

**Pass criterion:**
Full regression suite green. Paint canvas survives reboot, pixel-verified. Updated site visible in the on-OS browser.

---

## M23 — Image Viewer

**What it is:** A new WM app (`App::Viewer`) that opens any `.PNG` file from the FAT16 disk and displays it within the window. Navigate between images with left/right arrow keys.

**Scope:**
- PNG decoder already exists (`src/png.rs`) -- this is purely a new app shell
- Window title shows the filename
- Image centered and scaled to fit the window content area (nearest-neighbor scaling)
- Left/right arrow keys cycle through all `.PNG` files on the disk in alphabetical order
- Background fill for images smaller than the window
- `mkdisk.sh` pre-loads a sample image on the disk

**Pass criterion:**
Serial emits `VIEWER_OK`. Proof script pixel-verifies a known pixel from the sample image appears in the window content area, and that pressing the right arrow key changes the displayed image.

---

## M24 — Audio Player (WAV)

**What it is:** Native audio playback. A new WM app (`App::Audio`) that reads `.WAV` files from FAT16 and streams raw PCM to the Intel HDA audio device exposed by QEMU `virt`.

**Scope:**
- New driver: `src/hda.rs` -- Intel HDA controller init, output stream setup, DMA buffer ring
- WAV parser: reads RIFF header, validates PCM format (16-bit stereo 44100Hz), extracts raw sample data
- Audio kernel task: streams DMA buffer in chunks, refills on interrupt
- New `App::Audio` window: filename, play/pause button, elapsed seconds display
- `mkdisk.sh` generates a short test tone WAV (440Hz sine, 3 seconds)
- 44100Hz 16-bit stereo only -- no resampling or format conversion
- QEMU flags required: `-device intel-hda -device hda-output` added to `demo.sh` and `m24_test.sh`
- Stretch goal: MP3 decoding -- only attempt if WAV is clean and CPU headroom exists

**Pass criterion:**
Serial emits `AUDIO_OK`. Proof script verifies HDA init and audio task runs clean for the full 3-second test tone. Subjective proof: Henry hears the sine tone when running `demo.sh`.

---

## Hard constraints carried forward

- No TLS, no JS engine, no Linux/Windows binary loading — still out of scope
- All proof criteria must be *run and observed*, not assumed
- `gen` remains renamed to `tag` throughout net.rs (Rust 2024 reserved keyword)
- Single core only — no SMP
- `/compact` when context window fills; `PROGRESS.md` stays current
