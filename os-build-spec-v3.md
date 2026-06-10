# Veil OS — Build Spec v3 (M25–M27)

Extends `os-build-spec-v2.md`. Same rules: every milestone is gated on an
observed proof. Reality is the grader. Read `PROGRESS.md` before starting.

---

## M25 — Per-Visitor QEMU Instances + Random Usernames

**What it is:** The hosted demo at `os.henryratterman.com` spawns a fresh,
isolated QEMU instance for every visitor. Each instance boots with a randomly
generated username baked into its disk image (`USER.TXT` on FAT16). The Chat
app reads that file on first launch and uses it as the sender label instead of
"Me". Everyone in the chat room sees everyone else's username.

**Scope — session manager (host side):**
- Replace the single static QEMU process with a session manager script
  (`scripts/session_manager.py`). It listens on a Unix socket or HTTP port
  (e.g. 6091).
- When noVNC connects, the manager:
  1. Generates a random two-word username (adjective + animal, e.g.
     `crimson_wombat`, `lucky_moose`). Word lists baked into the script.
     Collision-check against active sessions.
  2. Calls `mkdisk.sh` with `--username <name>` to build a fresh `disk-<id>.img`
     with `USER.TXT` containing the username.
  3. Spawns a QEMU process on a free VNC port (`:11`, `:12`, ... up to 20
     concurrent sessions). Assigns a free websockify port (6100+).
  4. Returns a redirect to the per-session noVNC URL.
- Session lifetime: 30 minutes of inactivity or explicit close → QEMU killed,
  disk image deleted, port reclaimed.
- Max 20 concurrent sessions. If at cap: return a "full, try again in a moment"
  HTML page (no crashing).
- LaunchAgent `com.veil.sessions` replaces `com.veil.qemu` +
  `com.veil.websockify`. `com.veil.reset` deleted (sessions self-expire).
- `os.henryratterman.com` nginx/tunnel proxies `/session/` prefix to the
  session manager; bare `/` serves the noVNC landing redirect.

**Scope — kernel side:**
- `mkdisk.sh` accepts `--username <name>` flag. Writes it to `USER.TXT` on the
  FAT16 image (max 20 bytes, newline-terminated).
- `src/chat.rs` (or wherever App::Chat lives): on first open, read `USER.TXT`
  via the FAT16 driver. Store in a kernel global (`CHAT_USERNAME`). Use it as
  the sender prefix on every outgoing message: `crimson_wombat: hello`.
- Incoming messages from the relay are displayed verbatim (already contain the
  sender's username prefix from their own kernel).
- If `USER.TXT` is missing or empty, fall back to a random 6-hex-char ID
  generated from the hardware timer value at boot.

**Pass criterion:**
- `scripts/m25_test.sh`: spawn two isolated QEMU instances via the session
  manager with different usernames (`alpha_fox`, `beta_owl`). Drive Chat open
  on both. Inject a message on instance A; pixel-verify it appears in instance
  B's log prefixed with `alpha_fox:`. Serial from both emits `CHAT_OK`.
- Manual proof: open `os.henryratterman.com` in two browser tabs simultaneously.
  Each gets a different username visible in its Chat window. A message typed in
  one tab appears with the correct username in the other.

---

## M26 — Direct Messages + Online User List

**What it is:** The Chat app gains two features: (1) a live sidebar showing
everyone currently connected to the relay, and (2) the ability to send a DM
to a specific user. The relay server handles routing.

**Scope — relay protocol upgrade:**
The existing UDP relay (`scripts/hub.py` or whatever M20/M21 landed on) is
upgraded to a simple TCP relay with a thin protocol:

```
CLIENT → SERVER on connect:  HELLO <username>\n
SERVER → ALL clients:        JOIN <username>\n      (on new connection)
SERVER → ALL clients:        PART <username>\n      (on disconnect)
SERVER → ALL clients:        MSG <from> <to_or_*> <len>\n<bytes>
CLIENT → SERVER:             MSG <from> <to_or_*> <len>\n<bytes>
```

- `to` is `*` for public room, or a specific username for DM.
- Server forwards `*` messages to all connected clients.
- Server forwards DM messages only to the named recipient (and echoes back
  to sender so they see their own DM in the log).
- Server sends `JOIN`/`PART` events so clients can maintain a user list.
- TCP, not UDP. Port 7778 (leave 7777 for the old UDP relay if regression
  tests still use it).
- The relay runs as a Python script on this Mac mini
  (`scripts/relay.py`), LaunchAgent `com.veil.relay`, public via Cloudflare
  tunnel on a new subdomain `relay.henryratterman.com` (TCP passthrough or
  HTTP upgrade — pick the simpler one that works through Cloudflare).

**Scope — kernel/Chat app:**
- Switch `src/chat.rs` from UDP to TCP. Use the existing TCP stack.
- On connect: send `HELLO <username>`.
- On `JOIN`/`PART` events: update an in-memory user list.
- Render the user list as a right-side panel in the Chat window (fixed 80px
  wide, Barlow-style bitmap labels, green dot = online). Clicking a username
  opens a DM compose mode: input field prefixed with `@username`.
- DM messages appear in the log in a distinct color (use the terracotta palette
  already in the framebuffer).
- Public room messages and DMs share one scrollable log with visual separation.

**Pass criterion:**
- `scripts/m26_test.sh`: three instances connected to the relay. Instance A
  sends a public message — pixel-verify it appears in B and C. Instance A
  sends a DM to B — pixel-verify it appears in B but NOT in C. User list in
  all three windows shows all three usernames. Serial emits `DM_OK`.
- Manual proof: two browser tabs, DM from one to the other, public message
  visible in both.

---

## M27 — First-Boot Setup Screen (Username + Timezone)

**What it is:** On first boot (no `USER.TXT` on the FAT16 disk), the OS
presents a full-screen setup screen before showing the desktop. The user
types a username (max 20 chars) and selects a timezone offset (UTC-12 to
UTC+14 via arrow keys). Values are written to `USER.TXT` and `TZ.TXT`,
then the OS transitions to the normal desktop. On all subsequent boots the
setup screen is skipped.

**Scope — kernel:**
- New module `src/setup.rs` (`App::Setup` or a pre-WM full-screen mode).
- Triggered in `main.rs` boot sequence: after net init, before showing the
  desktop — check FAT16 for `USER.TXT`. If missing or empty, run setup.
- Setup screen layout:
  - Full screen, dark background (`#0b1018`), centered card (~400x300px).
  - Header: "Welcome to Veil OS" in the largest bitmap font available.
  - Field 1: "Your name" — blinking cursor, keyboard input, max 20 chars,
    live echo. Backspace works.
  - Field 2: "Timezone" — displays `UTC+0` by default, left/right arrows
    cycle through UTC-12 to UTC+14 in 30-min increments. Show current
    offset value as it changes.
  - Confirm button (or Enter key): validate name is non-empty, write
    `USER.TXT` and `TZ.TXT`, fade/wipe to desktop.
- After writing, the NTP-synced clock immediately shows the correct local
  time with the new offset.

**Scope — hosted demo:**
- Because each session manager instance has a fresh disk with no `USER.TXT`,
  every visitor will hit the setup screen on first boot. This is the
  intended UX: you land, name yourself, pick your timezone, enter the OS.
- The session manager no longer needs to bake a username into `USER.TXT`
  at disk-creation time (M25's `--username` flag becomes unused for hosted
  sessions). The kernel handles it. Keep the flag for test scripts.

**Pass criterion:**
- `scripts/m27_test.sh`: boot with a blank `USER.TXT` (or no file).
  Drive the setup screen: type `testuser`, arrow-key timezone to `UTC-5`,
  press Enter. Pixel-verify the desktop appears. Reboot. Pixel-verify setup
  screen does NOT appear. Verify `USER.TXT` contains `testuser` and `TZ.TXT`
  contains `-5`. Serial emits `SETUP_OK`.
- Manual proof: open `os.henryratterman.com` fresh tab, see setup screen,
  enter a name and timezone, reach the desktop. Open Chat — username shown
  correctly. Clock shows correct local time for chosen offset.

---

## M28 — Browser Audio via PCM-over-WebSocket

**What it is:** The Audio app's output becomes audible in the browser on
`os.henryratterman.com`. No GStreamer, no WebRTC stack required. QEMU writes
audio to a named FIFO pipe via `-audiodev wav,id=snd0,path=/tmp/veil-audio-<session>.fifo`;
a lightweight Node.js server (`scripts/audio_server.js`) tails that FIFO and
forwards raw PCM chunks over a per-session WebSocket. The noVNC page receives
the chunks and plays them via the Web Audio API.

**Why this approach:** The Homebrew QEMU on macOS only exposes `coreaudio`,
`dbus`, `wav`, and `none` audio backends -- no PipeWire, no GStreamer. The
`wav` backend can write to a named pipe (FIFO) instead of a file. That FIFO
becomes the audio tap with zero additional QEMU dependencies.

**Scope -- host side:**
- `scripts/audio_server.js` (Node.js, no npm deps beyond built-ins):
  - Accepts a `?session=<id>` WebSocket connection from the browser.
  - Opens the corresponding FIFO `/tmp/veil-audio-<id>.fifo` for reading.
  - Forwards raw PCM chunks (16-bit stereo 44100Hz, same format the kernel
    produces) to the WebSocket as binary frames. Chunk size: 4096 bytes
    (~23ms at 44100Hz stereo 16-bit).
  - Closes cleanly when the WebSocket disconnects or the FIFO EOF's.
- `scripts/serve_vnc.sh` / session manager: launch QEMU with:
  ```
  -audiodev wav,id=snd0,path=/tmp/veil-audio-<session>.fifo,out.try-poll=off
  -device virtio-sound-device,audiodev=snd0
  ```
  Create the FIFO (`mkfifo`) before launching QEMU.
- `audio_server.js` runs as LaunchAgent `com.veil.audio` on port 6092.
  Cloudflare tunnel ingress: `audio.henryratterman.com → localhost:6092`
  (WebSocket upgrade must be enabled in the ingress rule).

**Scope -- browser side (noVNC page patch):**
- Patch `index.html` (or a sidecar `audio.js` loaded from it) to:
  - On page load, open a WebSocket to
    `wss://audio.henryratterman.com?session=<id>` (session ID injected by
    the session manager into the noVNC page at serve time).
  - Create an `AudioContext` on first user gesture (browser autoplay policy
    requires this -- wire it to the first mouse click or keypress on the
    noVNC canvas).
  - On each binary WebSocket message: decode the raw PCM into an
    `AudioBuffer` (Int16Array, 2 channels, 44100Hz), schedule it for
    playback via `audioContext.createBufferSource()`. Queue buffers back-to-
    back to avoid gaps.
  - Show a small speaker icon overlay on the noVNC page: gray = connecting,
    green = playing, red = error. Clicking it toggles mute.

**Scope -- local `demo.sh`:**
- No change. Local runs use `-audiodev coreaudio` and hear audio natively
  through macOS. The FIFO path is only for the hosted headless sessions.

**Pass criterion:**
- `scripts/m28_test.sh`: start a headless QEMU session with the FIFO
  audiodev. Connect the audio WebSocket server to the FIFO. Send a
  "play" action to the Audio app via the proof driver. Read at least
  4096 bytes from the WebSocket within 5 seconds. Verify the bytes are
  non-zero (actual PCM, not silence). Emit `AUDIO_STREAM_OK` on serial.
- Manual proof: open `os.henryratterman.com`, complete setup screen,
  open the Audio app, click Play. Hear the 440Hz tone through the
  browser tab. Speaker icon turns green.

---

---

## M29 — In-OS File Manager (App::Files)

**What it is:** A new app (`App::Files`) that shows every file on the FAT16
disk in a scrollable list. Clicking a `.PNG` opens it in the Image Viewer.
Clicking a `.WAV` opens it in the Audio player. Clicking a `.TXT` opens it
in the Editor. Everything on the disk is discoverable and launchable from
one place.

**Scope -- kernel:**
- New `src/files.rs` implementing `App::Files`.
- Window: full list of files on FAT16, one per row. Each row shows filename
  and a type icon drawn with the bitmap font (e.g. `[IMG]`, `[WAV]`, `[TXT]`,
  `[???]` for unknown). Rows are 14px tall; list scrolls with up/down arrow
  keys or mouse wheel if more files than fit.
- Clicking a row (or pressing Enter on the selected row) dispatches to the
  correct app: PNG -> App::Viewer opens that file, WAV -> App::Audio opens
  that file, TXT -> App::Editor opens that file. Pass the filename as the
  initial file to open (each app already knows how to open a named file).
- Files app icon added to the taskbar (last position). Desktop icon grid
  entry added.
- No delete, rename, or write operations -- read-only browser for now.

**Scope -- mkdisk.sh:**
- Add support for a `user-files/` directory at the repo root. If the
  directory exists and contains `.png` or `.wav` files, copy them onto the
  FAT16 image alongside the built-in samples. Files are silently skipped if
  they would exceed the disk image size limit (32MB FAT16 cap).
- `run.sh` prints a hint after cloning: "Drop .png or .wav files into
  ~/veil-os/user-files/ and re-run to add them to the OS."
- Add `user-files/` to `.gitignore`.

**Pass criterion:**
- `scripts/m29_test.sh`: boot, open Files from taskbar. Pixel-verify the
  file list renders (at least one filename visible). Click the first PNG
  entry; pixel-verify the Viewer window opens with an image loaded. Serial
  emits `FILES_OK`.
- Manual proof: drop a custom PNG into `user-files/`, re-run `run.sh`,
  open Files app, click the PNG, see it in the Viewer.

---

## M30 — Pre-Boot File Upload on Hosted Demo

**What it is:** Visitors to `os.henryratterman.com` see a landing page
before the OS boots. They can drag-and-drop or click to upload `.png` and
`.wav` files (up to 5 files, 4MB each). When ready they click "Boot Veil OS"
-- the session manager bakes the uploaded files into the disk image before
spawning QEMU, so the files appear in the File Manager when the OS starts.

**Scope -- session manager (`scripts/session_manager.py`):**
- New HTTP routes (added to the existing session manager server):
  - `GET /` -- serves the landing page (`scripts/landing.html`).
  - `POST /upload?session=<id>` -- accepts multipart file upload. Validates:
    extension must be `.png` or `.wav`, size <= 4MB per file, max 5 files
    total. Stores uploads to `/tmp/veil-uploads-<id>/`.
  - `POST /boot?session=<id>` -- triggers disk image creation with uploaded
    files included, then spawns QEMU. Returns a redirect to the noVNC URL.
    If no uploads, boots with default disk (same as before).
- Session IDs are generated on `GET /` and embedded as a hidden field in
  the landing page form. The same ID is used for uploads and boot.

**Scope -- landing page (`scripts/landing.html`):**
- Dark page matching the OS aesthetic (`#0b1018` background, monospace font).
- Header: "Veil OS" + tagline "A toy operating system. Runs in your browser."
- Drop zone: dashed border box, "Drop .png or .wav files here (up to 5,
  4MB each)". Drag-and-drop and click-to-browse both work.
- File list: shows uploaded filenames with a remove button per file.
- "Boot Veil OS" button: disabled until at least the page loads (always
  bootable even with no uploads). Submits to `/boot?session=<id>`.
- No frameworks, vanilla JS only, inline CSS.

**Scope -- mkdisk.sh:**
- Accept an optional `--extra-dir <path>` flag. If provided, copies all
  `.png` and `.wav` files from that directory onto the FAT16 image. Used
  by the session manager after uploads land in `/tmp/veil-uploads-<id>/`.

**Scope -- run.sh (local):**
- After clone/update, if `user-files/` exists and has files, pass
  `--extra-dir user-files` to `mkdisk.sh` automatically. Print the hint
  about `user-files/` on first run if the directory doesn't exist yet.

**Pass criterion:**
- `scripts/m30_test.sh`: start the session manager. POST a test PNG to
  `/upload?session=test123`. POST to `/boot?session=test123`. Verify QEMU
  spawns. Boot the resulting instance and open the Files app -- pixel-verify
  the uploaded PNG filename appears in the file list. Emit `UPLOAD_OK`.
- Manual proof: visit `os.henryratterman.com`, upload a PNG, click Boot,
  open Files app in the OS, see the uploaded file, click it, see it in
  the Viewer.

---

## Hard constraints carried forward

- No TLS, no JS engine, no Linux/Windows binary loading
- All proof criteria must be run and observed, not assumed
- Single core only, no SMP
- `tag` not `gen` in net.rs (Rust 2024 reserved keyword)
- `/compact` when context window fills; `PROGRESS.md` stays current
- Self-hosted on Detroit Mac mini via Cloudflare tunnel — no DO droplet
- Babysitter cron permanently deleted — do not recreate
