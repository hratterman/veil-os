# Veil OS — Full Roadmap

> Built from scratch. AArch64. No dependencies. No ceiling.

---

## Completed

- **M1–M34**: Kernel, virtio drivers, FAT16, desktop, file manager, browser (TLS 1.3 from scratch, CSS, flexbox, fonts), GIF/PNG decoder, Lisp REPL with persistence, audio over WebSocket, per-visitor QEMU instances, upload page, 7-page in-OS internet, icon drag-and-drop
- **Bugfixes**: PNG OOM crash fixed with streaming downscale-on-decode; browser CSS engine upgraded (descendant selectors, @media, rem/em, contrast pass); henryratterman.com renders correctly

---

## Active

### M35 — "Go Crazy"
10 systems in one shot:
1. JPEG decoder (DCT, Huffman, YCbCr) — real web snaps into place
2. Browser text input (editable address bar, `<input>`, forms)
3. Copy/paste (global clipboard, Ctrl+C/V everywhere)
4. Real shell (ls/cat/cp/mv/rm/mkdir/pipes/history/tab-complete against FAT16)
5. App isolation + cooperative multitasking (per-app heap, kill without reboot, Alt+Tab)
6. GUI overhaul (dark/minimal/modern — rounded corners, shadows, redesigned icons, slim taskbar, animations)
7. Video playback (MJPEG at 24fps using JPEG decoder)
8. WASM runtime + AArch64 JIT (parse binary format, JIT-compile to native ARM64, run Mandelbrot/raytracer at near-native speed)
9. Virtio-net + full TCP/IP stack (Ethernet, ARP, IPv4, TCP, DNS — kernel owns the network, no host proxy)
10. Snake + Breakout games

---

## Upcoming

### M36 — "The App Platform"
Make WASM the primary app model. Veil becomes extensible without recompiling the kernel.
- WASM ABI spec: syscalls for drawing, keyboard input, file I/O, network (via kernel TCP)
- Rust + C SDK with templates and docs
- Hot-load: drop a .wasm onto the desktop, it runs as a first-class app with its own window
- App store page on os.henryratterman.com: upload .wasm, share a link, anyone can run it
- Port the built-in apps (Lisp REPL, Snake, Breakout) to WASM to prove the ABI
- Sandboxing: WASM linear memory is isolated, app can't corrupt kernel heap

### M37 — "Persistence + Identity"
Every visitor gets a persistent world. Demo becomes product.
- Per-user disk images on Detroit (keyed to cookie or login token)
- Simple account system: username + password, SQLite on host
- Your files, desktop layout, Lisp env, bookmarks — all there next visit
- Shareable desktops: `os.henryratterman.com?user=henry` loads your world
- Session resume: reconnect after disconnect, QEMU instance kept alive for 30 min
- Disk snapshots: save/restore named snapshots from inside the OS

### M38 — "Multiplayer"
Two people, one OS.
- Shared filesystem: opt-in shared FAT16 partition visible to multiple users
- Chat app (WASM): kernel TCP relay on Detroit, real-time messages between Veil sessions
- Collaborative Lisp REPL: shared environment, both users see each other's eval
- Spectate mode: watch another user's desktop live (second VNC stream, noVNC overlay)
- Presence: see who else is online, their username in the taskbar corner
- "Knock": request to join someone's session

### M39 — "The Showpiece"
Polish everything into a product worth showing.
- Landing page redesign: `os.henryratterman.com` becomes a real product page (what Veil is, the tech achievement, "try it" CTA, screenshot gallery)
- Boot animation: ASCII art splash → GUI splash → desktop (with real timing, not instant)
- Guided tour app: first-time visitor gets a walkthrough of what they're looking at and what's under the hood
- Release builds for visitors: kernel compiled with `--release`, 5-10x faster everything
- Technical writeup: blog post / GitHub README telling the full story — TLS from scratch, WASM JIT, TCP/IP, all of it
- Performance pass: profile the browser render loop and JPEG decoder, fix the hot paths

---

## The Ceiling (and how we smash through it)

### M40 — "JavaScript"
A minimal JS engine. Not V8. Just enough to make the modern web work.
- Lexer + recursive descent parser for ES5 subset
- Bytecode compiler + interpreter (or JIT-compile to WASM, then use the M35 WASM JIT)
- DOM bindings: `document.querySelector`, `getElementById`, `addEventListener`, `innerHTML`
- Enough to run: jQuery-free sites, basic SPAs, `henryratterman.com` hero section (currently JS-injected, shows blank)
- This is 2-4 weeks of Fable time. Worth it. Makes Veil render the real web.

### M41 — "GPU"
Hardware-accelerated rendering via virtio-gpu.
- virtio-gpu driver (virtio-mmio, same pattern as sound + net)
- OpenGL ES 2.0 subset via Gallium/virgl (QEMU provides the translation layer)
- Canvas API exposed to WASM apps: `canvas.fillRect`, `canvas.drawImage`, `canvas.arc`
- Smooth 60fps animations, WebGL-style graphics in WASM apps
- Port the browser compositor to GPU: hardware-accelerated text rendering, image scaling

### M42 — "SMP"
Multiple CPU cores. Real parallelism inside the kernel.
- QEMU already supports `-smp 4` for AArch64
- PSCI (Power State Coordination Interface): wake secondary cores via `CPU_ON`
- Per-core stacks, per-core interrupt handling
- Work-stealing scheduler: WASM apps and shell commands can run on separate cores
- Kernel remains single-threaded for simplicity (big kernel lock); only app execution parallelizes

### M43 — "Real Hardware"
Boot on a Raspberry Pi 5.
- The kernel is already AArch64 — the ISA is identical
- Swap QEMU's DTB for the Pi's device tree
- Replace virtio drivers with real hardware drivers: PL011 UART (already partially there), VC4/V3D GPU, USB HID for keyboard/mouse, SD card for FAT16
- Veil running on bare metal, no hypervisor, on a $80 computer
- Ship a flashable SD card image: `dd if=veil.img of=/dev/sdX`
- Demo: Veil booting on a Pi plugged into a monitor, zero software between the kernel and the hardware

### M44 — "The Network OS"
Veil as a peer in a real network.
- Full IPv6 support
- WebSocket server in the kernel: other browsers can connect to a running Veil instance
- Veil-to-Veil protocol: two Veil instances can share files and apps directly over TCP
- NAT traversal via the host (STUN/TURN on Detroit)
- "Veil Network": a small overlay network of running Veil instances, each visible to the others

---

## The Vision

Veil started as a from-scratch AArch64 OS to prove it could be done. It's already past that.

The trajectory: demo → platform → product → real hardware → networked OS.

Every milestone is a genuine engineering achievement. TLS 1.3 from scratch. WASM JIT from scratch. TCP/IP from scratch. JavaScript engine from scratch. GPU driver from scratch.

By M43, Veil is a real operating system running on real hardware with a real app ecosystem, a multiplayer web of connected instances, and a browser that renders the modern web.

That's not a demo. That's something.
