# Build Spec: A From-Scratch Graphical OS in Rust (AArch64)

**Codename:** your call.
**Target:** QEMU `virt` (AArch64) for development, Raspberry Pi **4** for the "it's real" payoff.
**Language:** Rust, `#![no_std]`, freestanding.
**Author of this spec:** written to be handed directly to a coding model as the contract for the whole project.

---

## 0. How to read this document

This is a contract, not a tutorial. It defines *what to build, in what order, and how each step is proven done.* The executing model must treat the milestone ladder (Section 5) as gated: **do not begin a milestone until the previous one provably passes its stated criterion in QEMU.** "Provably passes" always means *ran it and observed the specified output*, never "the code looks correct."

The hard rule of this project: **reality is the grader.** Pretty code that hasn't booted is worth nothing here. Every milestone ends with a concrete, observable pass condition.

---

## 1. What this is and is not

This OS will, when complete: boot on bare metal, manage physical and virtual memory, multitask, draw to a real framebuffer, run a windowing system with a mouse, run a Paint application, persist files, run a TCP/IP stack, serve HTTP to other machines on the network, and render a bounded slice of HTML/CSS in its own on-screen browser.

This OS will **not**: run Linux/Windows binaries, load arbitrary modern websites (no TLS gauntlet, no JavaScript engine, no full CSS), or drive arbitrary real hardware. These are out of scope **by design**, not by accident. The browser is a real browser for documents *we* serve, not a worse Chrome that pretends to load Gmail. Do not let scope creep reintroduce these.

If asked to do something in the "impossible for everyone" pile (break crypto, solve the halting problem, etc.), refuse and say why. That is a correctness property of the executor, not a limitation.

---

## 2. Hardware decision (read before buying anything)

**Develop on QEMU `virt`. Prove it on a Raspberry Pi 4.**

- The `virt` machine is a synthetic board with clean, well-documented devices and no legacy cruft. RAM begins at `0x4000_0000`. There is no firmware archaeology. This is where ~95% of development happens because iteration is instant and input/graphics/networking all "just work."
- **Use a Pi 4, not a Pi 5.** On the Pi 5, USB, Ethernet, and other I/O route through the RP1 southbridge over PCIe, which is effectively undocumented for bare-metal use, and even the firmware framebuffer path has known depth-reporting bugs. The Pi 4 has mature community documentation (rpi4-osdev, valvers, the BCM2711 peripheral manual) and a straightforward mailbox framebuffer. Choosing the Pi 5 adds months of reverse-engineering for zero pedagogical gain.
- Real-hardware honesty (applies to the Pi 4): **output is easy, input and networking are walls.** HDMI framebuffer via the mailbox is reachable. A USB keyboard/mouse requires a USB host stack + HID, which is brutal. Wired Ethernet is hard but tractable; Wi-Fi is a closed firmware blob and is out of scope. Therefore the plan lands the *interactive* graphical world fully in QEMU, and treats full interactivity + networking on the Pi as **stretch goals**. The Pi's guaranteed deliverable is: boots, puts our graphics on a real monitor.

---

## 3. Development environment (macOS, Apple Silicon)

Apple Silicon is itself ARM, so the toolchain stays in one family and you do **not** need a GNU cross-toolchain. Rust's bundled LLD linker (`rust-lld`) handles linking; this sidesteps the single most common macOS setup failure (fighting `aarch64-elf-binutils`).

```bash
# Prerequisites
xcode-select --install
brew install qemu          # provides qemu-system-aarch64; verify >= 8.x
brew install llvm          # optional: llvm-objdump/objcopy if not using cargo-binutils
rustup update
rustup target add aarch64-unknown-none
rustup component add llvm-tools-preview rust-src
cargo install cargo-binutils   # objcopy/size/nm via cargo
```

Toolchain notes the executor must respect:

- **Target triple:** `aarch64-unknown-none` (no OS, no std, soft-float-free hard target). Build with `cargo build --target aarch64-unknown-none`.
- **Linking:** use `rust-lld` via a `.cargo/config.toml` and a custom linker script. Do not invoke the host `ld`.
- **Debugging:** `gdb` is painful-to-impossible to target from Apple Silicon for cross debugging. Use **LLDB**, which speaks the gdb-remote protocol, against QEMU's stub (`-s -S`). This is the supported path on this machine; do not waste cycles trying to build a working cross-gdb.
- **Image vs ELF:** keep both. QEMU can boot the ELF directly via `-kernel`. For the Pi you will produce a raw binary (`kernel8.img`) via `cargo objcopy`.

---

## 4. The QEMU `virt` platform reference (authoritative addresses)

The executor must hard-code only the addresses QEMU guarantees, and read everything else from the **device tree blob (DTB)**, which on a bare-metal boot QEMU places at the start of RAM (`0x4000_0000`).

Guaranteed / conventional facts for `qemu-system-aarch64 -machine virt`:

- **RAM base:** `0x4000_0000`. Load the kernel at `0x4010_0000` by convention.
- **PL011 UART0 (serial):** MMIO at `0x0900_0000`. This is your first and most important output device.
- **Interrupt controller:** GIC (v2 by default). Timer and external interrupts route through it.
- **CPU:** none is implied for AArch64 — you **must** pass `-cpu` (use `cortex-a72`, matching the Pi 4's cores, so code is closer to portable).
- **Graphics:** no default display. Add `-device ramfb` and discover/configure it via **fw_cfg** (the fw_cfg MMIO region's location is in the DTB). The legacy VGA linear framebuffer does **not** work on aarch64 `virt`. `ramfb` is the simple path; `virtio-gpu-pci` is the proper-but-harder upgrade.
- **Networking:** `virtio-net-pci` (or `-mmio`) for the NIC.
- **Block storage:** `virtio-blk` for the disk/filesystem backing.
- **Exception level:** on `virt` the kernel typically enters at EL2 (or EL1). The boot code must detect its EL and drop cleanly to EL1 before setting up the kernel proper. Do not assume EL1 on entry.

Everything not in that list (device addresses, IRQ numbers, RAM size) **must** be parsed from the DTB, not guessed. Hard-coding them is a latent portability bug the executor should refuse to introduce.

---

## 5. The milestone ladder

Each milestone has: **Goal**, **Build**, **Pass criterion** (the observable proof), and **Where it bites** (the failure mode to expect). The ladder is ordered so that the first visible "whoa" (pixels) comes early, and deep systems work earns its place under something you can see.

### Phase A — Bring-up

**M1. Boot to serial.**
- *Goal:* a freestanding kernel boots and speaks.
- *Build:* `.cargo/config.toml`, linker script placing `_start` at `0x4010_0000`, boot assembly (set up a stack, zero `.bss`, detect EL and drop to EL1, jump to Rust), a `panic_handler`, and a PL011 UART driver (just polled TX is enough).
- *Pass:* QEMU prints a known sentinel line (e.g. `BOOT_OK: <name> kernel alive`) on the serial console.
- *Where it bites:* wrong load address, stack not set up, `.bss` not zeroed, linker script section ordering. Classic symptom: total silence, no error. Use `-d int,cpu_reset` and `-s -S` + LLDB to single-step `_start`.

**M2. Exceptions + timer.**
- *Goal:* the CPU can trap and recover, and time passes.
- *Build:* the AArch64 exception vector table (`VBAR_EL1`), handlers that print the trap cause + faulting address, GIC initialization, and the architectural generic timer programmed to fire periodically.
- *Pass:* (a) deliberately trigger a synchronous exception and watch the handler print it instead of dying; (b) a timer tick counter visibly increments on the serial console.
- *Where it bites:* vector table alignment (2KB-aligned, exact 16-entry layout), `DAIF` interrupt masking, GIC distributor/CPU-interface init, EL1 vs EL2 timer registers. Symptom: silent hang or recursive fault loop. This is the first real debugging cliff.

### Phase B — Memory

**M3. Physical memory + paging.**
- *Goal:* manage RAM and turn on the MMU.
- *Build:* parse the DTB to learn RAM size; a physical frame allocator (bitmap or free-list); page tables (4KB granule, the standard AArch64 multi-level walk); set `TTBR0_EL1`/`TCR_EL1`/`MAIR_EL1` and enable the MMU; map a fresh virtual region.
- *Pass:* write a sentinel value through a freshly mapped virtual address, read it back correctly **with the MMU on**, and print it.
- *Where it bites:* descriptor bit layout, memory attribute indices (MAIR), cache/coherency settings, TLB invalidation, and the dangerous moment of enabling the MMU (the instruction stream's own mapping must be valid or you fault immediately). Expect to single-step the MMU-enable in LLDB.

**M4. Kernel heap.**
- *Goal:* `alloc` works.
- *Build:* a `#[global_allocator]` (a linked-list or buddy allocator over a heap region carved from physical memory) so `alloc::boxed::Box`, `Vec`, `String`, `BTreeMap` work.
- *Pass:* a stress loop that allocates and frees thousands of varied-size blocks, interleaved, and asserts no corruption and no monotonic leak (print free-bytes before/after). 

### Phase C — Pixels (the first reward)

**M5. Framebuffer: pixels, lines, text.**
- *Goal:* the screen lights up.
- *Build (QEMU):* implement the **fw_cfg** DMA interface, find the `ramfb` entry, configure it (width, height, `XRGB8888`, stride), and obtain a linear framebuffer pointer. Then a tiny 2D library: `put_pixel`, `fill_rect`, `draw_line`, and a bitmap font blitter (`draw_char`/`draw_string`) using an embedded 8x16 font.
- *Pass:* QEMU's display window shows a filled background, some lines/rectangles, and a line of text rendered by your font blitter. **First visual milestone — screenshot it.**
- *Where it bites:* fw_cfg DMA register layout and endianness, selecting the right entry, stride vs width, and pixel format byte order. If stuck, study a known-good bare-metal ramfb implementation for the fw_cfg handshake, then write your own.

**M6. Input: keyboard + mouse.**
- *Goal:* the system reacts to you.
- *Build (QEMU):* a driver for the input devices QEMU exposes (`virtio-input`, or PL050 PS/2 on some setups — confirm via DTB / `-device` choice; prefer `virtio-keyboard-device` + `virtio-mouse-device` or `usb-kbd`/`usb-tablet` via `qemu-xhci`). Maintain a key event queue and a mouse position + button state.
- *Pass:* typed keys echo to the framebuffer; moving the mouse moves an on-screen cursor; clicks are detected.
- *Where it bites:* virtio queue setup (descriptor rings) if going the virtio route; this is your first taste of the virtio protocol you'll reuse for net and block. Symptom: events never arrive. Verify the queue notify path.

### Phase D — The graphical shell

**M7. Window manager.**
- *Goal:* overlapping, draggable windows.
- *Build:* a window abstraction (title bar, content rect, z-order), a compositor that redraws dirty regions back-to-front, mouse-driven focus and dragging, and double-buffering to kill flicker.
- *Pass:* two or more windows on screen; dragging one by its title bar moves it smoothly over the other; clicking changes focus/z-order.
- *Where it bites:* dirty-rectangle tracking and tearing. Naive full-redraws will work first; optimize only after correct.

**M8. Paint.**
- *Goal:* the headline fun app.
- *Build:* a Paint application living in a window — freehand brush following the mouse, a small color palette, adjustable brush size, a clear/fill action, and a canvas buffer distinct from the window chrome. Optional: shapes (line/rect/ellipse), eraser, fill bucket.
- *Pass:* you draw with the mouse inside the window, change colors and brush size, and the strokes persist on the canvas as you move other windows over and away from it.

### Phase E — Real OS underneath

**M9. User mode + syscalls.**
- *Goal:* the privilege boundary — the difficulty cliff.
- *Build:* load a separate program into a user address space, drop to EL0, and implement a syscall path (`SVC` instruction → synchronous exception → kernel handler → return). Start with `write`, `exit`, and something for drawing/event access so user programs can use the GUI.
- *Pass:* a *separately built* user binary runs in EL0 and makes a syscall that the kernel services (e.g. prints, or draws into its window), then returns cleanly to user code.
- *Where it bites:* register save/restore in the trap path, `SPSR`/`ELR` handling, separate user/kernel stacks, the user page mappings, and `SP_EL0` vs `SP_EL1`. This and M2 are the two places the executor must debug from almost no signal. Budget for it.

**M10. Filesystem.**
- *Goal:* persistence.
- *Build:* a `virtio-blk` driver, and a simple filesystem (FAT16/32 for Pi-SD compatibility later, or a clean custom FS for simplicity). Operations: list directory, read file, write/create file.
- *Pass:* save a Paint canvas to a file, reboot the OS, load it back, and see the same image. Also: list files from the shell.

**M11. Shell + multitasking.**
- *Goal:* it feels like a system.
- *Build:* preemptive multitasking (the timer interrupt switches between tasks — reuse M2 + M9), a scheduler, and a text shell (in a window) that parses commands, loads user binaries from the FS (M10), and runs them in user mode. Implement `ls`, `cat`, `echo`, and launching Paint.
- *Pass:* from the shell, `ls` lists files, `cat` prints one, and launching Paint opens it as a running task while the shell stays responsive. Stretch: pipes (`ls | grep x`), which proves processes + file descriptors + IPC compose.
- *Where it bites:* context-switch assembly (saving/restoring the full register set + `SP`/`PC`/`PSTATE`). Corruption here manifests as random crashes much later — fuzz it by forcing rapid switches.

### Phase F — The network crescendo (the part that shines)

This is the headline. It makes the OS touch the outside world with no asterisk. Reference `smoltcp` (a real `no_std` Rust TCP/IP stack) for study; you may either reimplement or integrate it, but the *web server on top must be yours*.

**M12. NIC driver: raw packets.**
- *Build:* a `virtio-net` driver (reusing the virtio machinery from M6). Send and receive raw Ethernet frames.
- *Pass:* the OS emits a hand-crafted Ethernet frame that your Mac (or `tcpdump` on the host tap) observes; and the OS prints the bytes of a frame it receives.

**M13. ARP + IPv4 + ICMP.**
- *Build:* ARP (resolve/cache), IPv4 parsing/emission with checksums, ICMP echo.
- *Pass:* **your Mac runs `ping <os-ip>` and the OS replies.** First "it's on the network" moment.

**M14. UDP, then TCP.**
- *Build:* UDP first (easy, connectionless). Then the TCP state machine: handshake (SYN/SYN-ACK/ACK), sequence/ack numbers, windowing, retransmission, teardown. This is the hard, beautiful core of the whole project.
- *Pass:* a TCP connection from your Mac (`nc <os-ip> <port>`) establishes, exchanges bytes both directions, and closes cleanly. Verify with packet capture that the handshake and teardown are correct.

**M15. HTTP server.**
- *Build:* an HTTP/1.1 server running as a task on your OS — parse the request line + headers, serve a small site (HTML/CSS/images) from the M10 filesystem, set correct `Content-Type` and `Content-Length`.
- *Pass:* **open your Mac's real browser, hit `http://<os-ip>:<port>/`, and load a page served by your operating system off a TCP stack you wrote.** This is the standout achievement.

### Phase G — Closing the loop

**M16. On-OS browser (the document slice).**
- *Goal:* a real browser for a bounded web — not a worse Chrome, a complete small one.
- *Scope (hold this line):* HTTP only (no TLS), no JavaScript. Support a deliberately bounded slice of HTML — block and inline elements: `html/head/body`, `h1`–`h6`, `p`, `a`, `ul/ol/li`, `img`, `div/span`, `br`, `pre`, plus a small CSS subset: `color`, `background-color`, `font-size`, `margin`, `padding`, `display: block|inline`, and maybe `width`. That's it. Document the exact supported grammar.
- *Build:* an HTML tokenizer + parser → DOM tree; a tiny CSS parser → a flat style map; a block/inline layout engine producing positioned boxes; a painter that draws boxes + text (reuse M5 font blitter) into a window (reuse M7); clickable `<a href>` that navigates by fetching the next page over your own TCP stack (M14) from your own server (M15).
- *Pass:* the OS's browser, in a window, fetches a multi-page hand-authored site **from your own HTTP server**, lays it out, renders text/headings/links/images, and clicking a link navigates to the next page. The full web loop — client and server, both halves yours, running on hardware your software drives.
- *Where it bites:* layout correctness has almost no error signal (the page renders "90% right" and you eyeball the rest). Keep the supported subset tiny and conformant rather than broad and buggy. **Do not** expand scope to chase real sites.

### Phase H — Make it real

**M17. Boot on the Raspberry Pi 4.**
- *Goal:* the same OS, on metal, on a real monitor.
- *Build:* a Pi 4 boot path — produce `kernel8.img` via `cargo objcopy`, a `config.txt` (`arm_64bit=1`, `kernel=kernel8.img`, UART enable for debugging), the Linux-style kernel header so the Pi bootloader loads it, and a Pi-specific framebuffer driver via the **VideoCore mailbox** (request a framebuffer, get the address/pitch). Replace the PL011 base/clock as needed for the BCM2711.
- *Pass (guaranteed deliverable):* the Pi 4 boots your kernel and renders your graphics (the windowing system / Paint canvas, even if static) on a real HDMI monitor. UART-over-GPIO gives you serial debugging on metal.
- *Stretch (the genuine walls):* USB HID for real keyboard/mouse input (USB host stack + HID — very hard), and wired Ethernet for the network stack on metal (hard; the gigabit MAC). Wi-Fi is out (closed blob). Treat these as glory, not blockers.

---

## 6. The test harness (build this in M1, use it forever)

Automated pass/fail is what makes the gating real. Set this up at the start.

- **Headless run:** pipe serial to a checker script that greps for each milestone's sentinel string and exits non-zero on timeout.
- **Self-reported exit:** have the kernel signal completion/failure so the runner gets a real exit code. On `virt`, use **semihosting** (`-semihosting`) to call `SYS_EXIT`, or wire a known MMIO "exit" convention.
- **Fault visibility:** always run with `-no-reboot -no-shutdown -d int,cpu_reset` during bring-up so a fault halts and dumps the trap instead of silently reboot-looping (which destroys the evidence).
- **Single-step:** keep `-s -S` + LLDB ready for the M2/M9 cliffs.
- **Adversarial grading (once M15/M16 work):** fuzz the syscall interface and HTTP parser with malformed input, fault-inject the allocator, and require the kernel to survive (no crash, graceful error). Long-horizon self-correction under an objective grader is the real test of the model.

---

## 7. Example QEMU command lines

Bring-up (M1–M2), serial only, fault-visible:

```bash
qemu-system-aarch64 \
  -machine virt -cpu cortex-a72 -m 512M \
  -nographic \
  -no-reboot -no-shutdown -d int,cpu_reset \
  -semihosting \
  -kernel target/aarch64-unknown-none/debug/<kernel>
```

Graphics + input (M5+):

```bash
qemu-system-aarch64 \
  -machine virt -cpu cortex-a72 -m 512M \
  -device ramfb \
  -device qemu-xhci -device usb-kbd -device usb-tablet \
  -serial mon:stdio \
  -kernel target/aarch64-unknown-none/debug/<kernel>
```

Full system: graphics + input + disk + network with port-forward (M10–M16):

```bash
qemu-system-aarch64 \
  -machine virt -cpu cortex-a72 -m 512M \
  -device ramfb \
  -device qemu-xhci -device usb-kbd -device usb-tablet \
  -drive if=none,file=disk.img,format=raw,id=hd0 \
  -device virtio-blk-pci,drive=hd0 \
  -netdev user,id=net0,hostfwd=tcp::8080-:80 \
  -device virtio-net-pci,netdev=net0 \
  -serial mon:stdio \
  -kernel target/aarch64-unknown-none/debug/<kernel>
```

With `hostfwd=tcp::8080-:80`, your Mac reaches the OS's port 80 at `http://localhost:8080`. For your Mac to `ping` the OS directly (M13), switch from user-mode networking to a **TAP** interface (more setup; document it when you get there).

Debug (attach LLDB):

```bash
# add -s -S to any of the above, then in another terminal:
lldb
(lldb) gdb-remote localhost:1234
```

---

## 8. Reference implementations to study (not copy)

The executor should read these for the *handshake-level* details that are hard to get from prose, then write its own code:

- **xv6-riscv** — the canonical small teaching OS; mirror its *shape* (process model, syscalls, FS) even though it's RISC-V.
- **Writing an OS in Rust** (Philipp Oppermann) — `no_std`, allocator, interrupts patterns (x86 but the Rust scaffolding transfers).
- **rust-raspberrypi-OS-tutorials** — AArch64-specific bring-up, EL handling, MMU, drivers; the closest match to this project's architecture.
- **rpi4-osdev / valvers** — Pi 4 mailbox framebuffer and bare-metal specifics for M17.
- **smoltcp** — reference `no_std` TCP/IP stack for M12–M14.
- **QEMU fw_cfg + ramfb docs/examples** — for the M5 framebuffer handshake.

---

## 9. Execution protocol (the contract for the model)

Paste this as the operating instruction:

> You are building the OS specified in this document. Rules:
> 1. Work the milestones M1→M17 **in order**. Do not start a milestone until the previous one **provably passes** its stated pass criterion.
> 2. "Provably passes" means you **ran it in QEMU and observed the specified output** — serial sentinel, on-screen result, ping reply, HTTP response, etc. Never report a milestone done from code inspection alone.
> 3. When it faults, hangs, or reboot-loops, **debug it yourself**: use `-d int,cpu_reset` for trap dumps and `-s -S` + LLDB to single-step. Reason from the actual fault. Do not guess-and-rewrite.
> 4. After each milestone, state: what you built, the exact QEMU command you ran, and the observed output that proves the pass criterion.
> 5. Hold the scope lines in Sections 1 and 16. No TLS, no JS, no real-site rendering. If you catch yourself reframing the scope to make a request "work," stop and flag it.
> 6. Parse the DTB for device info; hard-code only the addresses Section 4 guarantees.
>
> Begin with M1: project scaffolding (`.cargo/config.toml`, linker script, boot assembly, panic handler, PL011 driver) and the test harness from Section 6. Show me the QEMU command and the serial output proving `BOOT_OK`.

---

## 10. Honest expectation-setting

M1–M8 (boot through Paint) is a focused, deeply satisfying stretch with frequent visible wins. M9 (user mode) and M14 (TCP) are the two genuine difficulty cliffs — expect the model to grind there, and that's exactly the part worth watching. M15 (serving HTTP to your real browser) is the payoff that has no asterisk. M16 (the browser) is real but only if the scope stays tiny. M17 on the Pi 4 reliably delivers "my OS on a real monitor"; full input + networking on metal are honest stretch goals, not promises.

Nothing here is frontier-impossible. All of it is "normally takes a team a long time," which is the right kind of hard for what you're testing. The thing that will distinguish a genuinely exceptional executor from a merely fluent one is **Section 6's adversarial grading and the M2/M9/M14 debugging** — long-horizon self-correction against a grader that can't be charmed by good code.
