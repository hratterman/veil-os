//! Veil — a from-scratch graphical OS for AArch64 (QEMU virt / Raspberry Pi 4).
//! Milestones passed: M1 (serial), M2 (exceptions + timer).
//! Current: M3 (memory + MMU), M4 (heap), M5 (framebuffer).
//!
//! Every boot re-runs each milestone's proof in order and prints its
//! sentinel — the harness greps the newest one, the older ones are
//! regression checks for free.

#![no_std]
#![no_main]

extern crate alloc;

mod blk;
mod browser;
mod clock;
mod crypto;
mod css;
mod desktop;
mod dtb;
mod exceptions;
mod fb;
mod files;
mod font;
mod frames;
mod fs;
mod fwcfg;
mod gic;
mod gif;
mod gifplayer;
mod heap;
mod html;
mod http;
mod input;
mod keymap;
mod lisp;
#[cfg(feature = "pi4")]
mod mbox;
mod net;
mod netdev;
mod paging;
#[cfg(feature = "pi4")]
mod pi4;
mod png;
mod repl;
mod scheduler;
mod semihosting;
mod setup;
mod snd;
mod syscall;
mod timer;
mod uart;
mod viewer;
mod virtio;
mod wm;

use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

core::arch::global_asm!(include_str!("boot.s"));
#[cfg(feature = "pi4")]
core::arch::global_asm!(include_str!("pi4_head.s"));

/// QEMU virt RAM base — where the DTB lives on an ELF `-kernel` boot
/// (x0 is only populated for Linux-format kernel images).
const RAM_BASE: usize = 0x4000_0000;

unsafe extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

/// Called from boot.s on core 0, at EL1, stack up, .bss zeroed.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_main(dtb_ptr: *const u8) -> ! {
    #[cfg(feature = "pi4")]
    pi4::main(dtb_ptr);

    #[cfg(not(feature = "pi4"))]
    virt_main(dtb_ptr)
}

/// The QEMU virt path: every milestone proof in order, then the desktop.
#[cfg(not(feature = "pi4"))]
fn virt_main(dtb_ptr: *const u8) -> ! {
    let serial = unsafe { uart::Pl011::new(uart::UART0_BASE) };
    serial.init();
    serial.put_str("BOOT_OK: veil kernel alive\n");

    let dtb_ptr = if dtb_ptr.is_null() { RAM_BASE as *const u8 } else { dtb_ptr };
    let fdt = unsafe { dtb::Fdt::new(dtb_ptr) }.expect("no DTB magic at RAM base");

    milestone2(&fdt);
    let kernel_mapper = milestone3(&fdt, dtb_ptr as usize);
    milestone4();
    scheduler::init(kernel_mapper.root());
    crypto::selftest(); // M33: prove SHA256/HKDF/ChaCha20-Poly1305/X25519 vectors
    milestone9();
    milestone10(&fdt);
    if milestone12(&fdt) {
        // M14/M15 services (tcp echo, http) as a preemptible kernel task.
        scheduler::spawn_kernel("net-services", http::services_task);
        milestone19b();
        if read_mode(&fdt).as_deref() == Some(b"net") {
            // Headless serving mode: stay alive, everything is IRQ-driven.
            kprintln!("NET_READY: serving icmp echo, udp echo :7, tcp echo :7777, http :80");
            timer::set_quiet(true);
            timer::start(timer::intid(), 50);
            scheduler::enable_preemption();
            loop {
                unsafe { core::arch::asm!("wfi") };
            }
        }
    }

    milestone24(&fdt);

    if let Some((screen, scene_mode)) = milestone5(&fdt) {
        if scene_mode {
            // M5 proof mode: leave the test scene up for the screendump;
            // QEMU is shut down from outside (QMP quit).
            loop {
                unsafe { core::arch::asm!("wfi") };
            }
        }
        desktop::run(screen, &fdt); // returns only if no input devices
    }
    kprintln!("All milestone checks passed; exiting.");
    semihosting::exit(0)
}

/// M9: a separately built binary runs at EL0 and syscalls back in.
/// hello.bin is compiled from the user/ crate (its own linker script,
/// its own build) and embedded so this proof also runs diskless.
fn milestone9() {
    static HELLO: &[u8] =
        include_bytes!("../user/target/aarch64-unknown-none/release/hello.bin");
    let pid = scheduler::spawn(HELLO, "hello", "").expect("spawn failed");
    kprintln!(
        "USER: spawned pid {pid}, {} byte image at {:#x}, dropping to EL0",
        HELLO.len(),
        scheduler::USER_BASE
    );
    scheduler::run_until_idle();
    let (reaped, code) = scheduler::reap().expect("hello never exited");
    assert_eq!(reaped, pid);
    assert_eq!(code, 0, "hello exited with nonzero code");
    kprintln!("USER_OK: pid {pid} ran at EL0, syscalled, exited cleanly (code {code})");

    // Negative proof: EL0 touching kernel memory must die, not succeed.
    static EVIL: &[u8] =
        include_bytes!("../user/target/aarch64-unknown-none/release/evil.bin");
    let evil_pid = scheduler::spawn(EVIL, "evil", "").expect("spawn failed");
    scheduler::run_until_idle();
    let (reaped, code) = scheduler::reap().expect("evil never died");
    assert_eq!(reaped, evil_pid);
    assert_eq!(code, 139, "evil read kernel memory without faulting!");
    kprintln!("USER_OK: kernel memory access from EL0 was fatal (code {code})");
    kprintln!("M9_OK");
}

/// M10: virtio-blk + FAT16. Headless proof: mount, list the root dir,
/// read a file, and persist a boot counter (write + read-back across
/// boots). The Paint-canvas half of the criterion is in the GUI test.
fn milestone10(fdt: &dtb::Fdt) {
    let Some(capacity) = blk::init(fdt) else {
        kprintln!("BLK_SKIP: no virtio-blk device");
        return;
    };
    kprintln!("BLK_OK: virtio-blk, {capacity} sectors ({} MiB)", capacity * 512 >> 20);
    if !fs::mount() {
        panic!("disk attached but FAT16 mount failed");
    }
    let entries = fs::list_root().unwrap_or_default();
    kprintln!("FS_OK: FAT16 mounted, {} root entries", entries.len());
    for (name, size) in &entries {
        kprintln!("FS_LS: {name} {size}");
    }
    if let Some(data) = fs::read_file("README.TXT") {
        let text = core::str::from_utf8(&data).unwrap_or("<binary>");
        kprintln!("FS_READ: README.TXT ({} bytes): {}", data.len(), text.trim_end());
    }
    // Write + persistence proof: a counter that survives reboots.
    let count = fs::read_file("BOOTCNT.TXT")
        .and_then(|d| core::str::from_utf8(&d).ok()?.trim().parse::<u32>().ok())
        .unwrap_or(0)
        + 1;
    let mut text = alloc::string::String::new();
    let _ = core::fmt::write(&mut text, format_args!("{count}"));
    fs::write_file("BOOTCNT.TXT", text.as_bytes()).expect("boot counter write failed");
    kprintln!("FS_WRITE: boot #{count} recorded in BOOTCNT.TXT");
    kprintln!("M10_OK");
}

/// The opt/veil.mode fw_cfg string, if QEMU was started with one.
fn read_mode(fdt: &dtb::Fdt) -> Option<alloc::vec::Vec<u8>> {
    let fw = fwcfg::FwCfg::from_dtb(fdt)?;
    let f = fw.find_file("opt/veil.mode")?;
    let mut buf = [0u8; 16];
    let n = fw.read_file(f, &mut buf).ok()?;
    Some(buf[..n].to_vec())
}

/// M12: virtio-net up; emit a hand-crafted raw Ethernet frame (the host
/// capture proves it left the machine) and print the bytes of a frame we
/// receive (provoked by ARP-probing the gateway — whoever answers, slirp
/// or a raw-socket test peer, is our RX proof). Returns false (skip) when
/// no NIC is attached so diskless/netless regressions stay green.
fn milestone12(fdt: &dtb::Fdt) -> bool {
    let Some(mac) = netdev::init(fdt) else {
        kprintln!("NET_SKIP: no virtio-net device");
        return false;
    };

    // IP config from fw_cfg opt/veil.net ("a.b.c.d/len,gw"), defaulting to
    // QEMU slirp's conventional layout.
    let mut cfg = ([10, 0, 2, 15], 24, [10, 0, 2, 2]);
    if let Some(fw) = fwcfg::FwCfg::from_dtb(fdt) {
        if let Some(f) = fw.find_file("opt/veil.net") {
            let mut buf = [0u8; 64];
            if let Ok(n) = fw.read_file(f, &mut buf) {
                if let Some(parsed) =
                    core::str::from_utf8(&buf[..n]).ok().and_then(net::parse_config)
                {
                    cfg = parsed;
                }
            }
        }
    }
    net::init(mac, cfg.0, cfg.1, cfg.2);

    // M26: optional chat relay address (fw_cfg opt/veil.relay = "ip:port").
    // Present -> the Chat app uses the TCP relay (DMs + user list); absent
    // -> it stays in the M20 UDP-broadcast mode. The self-hosted demo sets
    // it to the slirp host gateway (10.0.2.2:7778) where relay.py runs.
    if let Some(fw) = fwcfg::FwCfg::from_dtb(fdt) {
        if let Some(f) = fw.find_file("opt/veil.relay") {
            let mut buf = [0u8; 64];
            if let Ok(n) = fw.read_file(f, &mut buf) {
                if let Some(addr) =
                    core::str::from_utf8(&buf[..n]).ok().and_then(net::parse_relay)
                {
                    net::set_relay(Some(addr));
                    kprintln!("RELAY: chat relay at {}:{}", net::fmt_ip(&addr.0), addr.1);
                }
            }
        }
    }

    // TX proof: a hand-crafted broadcast frame with an experimental
    // ethertype and a greppable payload.
    let mut frame = [0u8; 60];
    frame[0..6].copy_from_slice(&[0xff; 6]);
    frame[6..12].copy_from_slice(&mac);
    frame[12..14].copy_from_slice(&0x88b5u16.to_be_bytes());
    frame[14..14 + 28].copy_from_slice(b"VEIL M12 raw ethernet frame!");
    netdev::send(&frame);
    kprintln!("NET_TX: 60-byte hand-crafted frame sent (ethertype 0x88b5, payload \"VEIL M12...\")");

    // RX proof: ask the network a question and wait for any frame back.
    net::arp_probe(cfg.2);
    let deadline = {
        let now: u64;
        unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) now, options(nomem, nostack)) };
        now + 2 * timer::frequency() // 2 s
    };
    loop {
        if net::rx_count() > 0 {
            kprintln!("M12_OK");
            break;
        }
        let now: u64;
        unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) now, options(nomem, nostack)) };
        if now > deadline {
            kprintln!("NET_RX_TIMEOUT: nothing answered our ARP probe (no peer on this netdev?)");
            break;
        }
    }
    true
}

/// M19b: anchor the wall clock to real time. Read TZ.TXT (an integer UTC
/// offset in hours) for the local timezone, then do one NTP exchange to
/// pool.ntp.org. On success the clock app shows real local time; on any
/// failure (no DNS, unreachable) it falls back to time-since-boot. Only
/// reached when a NIC is present (milestone12 returned true).
fn milestone19b() {
    // Timezone offset from TZ.TXT (e.g. "-5" => EST, "5.5" => IST). The
    // first-boot setup screen (M27) writes this; default UTC if absent.
    let tz_secs = fs::read_file("TZ.TXT")
        .and_then(|d| parse_tz_offset(core::str::from_utf8(&d).ok()?.trim()))
        .unwrap_or(0);
    timer::set_tz(tz_secs);
    kprintln!("TZ: UTC offset {tz_secs}s (from TZ.TXT)");

    match net::ntp_sync("pool.ntp.org") {
        Some(unix) => {
            timer::set_wall(unix);
            kprintln!("NTP: set clock to {unix}");
            kprintln!("M19b_OK");
        }
        None => kprintln!("NTP: no sync (network unreachable); clock uses time-since-boot"),
    }
}

/// Parse a TZ.TXT offset string ("[+-]hours[.5]") into seconds.
fn parse_tz_offset(s: &str) -> Option<i64> {
    let neg = s.starts_with('-');
    let body = s.trim_start_matches(['+', '-']);
    let (hours, half) = match body.split_once('.') {
        Some((h, frac)) => (h.parse::<i64>().ok()?, if frac.starts_with('5') { 1 } else { 0 }),
        None => (body.parse::<i64>().ok()?, 0),
    };
    let secs = hours * 3600 + half * 1800;
    Some(if neg { -secs } else { secs })
}

/// M24: virtio-sound bring-up. Initializes the device if present (the
/// App::Audio window then plays via a kernel task). In `opt/veil.mode=audio`
/// it plays the on-disk test tone synchronously and emits AUDIO_OK — the
/// headless proof path.
fn milestone24(fdt: &dtb::Fdt) {
    if !snd::init(fdt) {
        kprintln!("SND_SKIP: no virtio-sound device");
        return;
    }
    kprintln!("SND_OK: virtio-sound output ready (44100 Hz, 16-bit stereo)");
    if read_mode(fdt).as_deref() == Some(b"audio") {
        snd::play_file("TONE.WAV");
    }
}

/// M5: fw_cfg handshake, ramfb framebuffer, pixels/lines/text on screen.
/// Returns None (skip, not failure) when no ramfb device is attached, so
/// headless harness runs of M1-M4 stay green. The bool is "m5scene" mode
/// (set via -fw_cfg name=opt/veil.mode,string=m5scene): keep the M5 test
/// scene on screen instead of starting the desktop.
fn milestone5(fdt: &dtb::Fdt) -> Option<(fb::Framebuffer, bool)> {
    const W: usize = 1024;
    const H: usize = 768;

    let Some(fw) = fwcfg::FwCfg::from_dtb(fdt) else {
        kprintln!("FB_SKIP: no fw_cfg device in DTB");
        return None;
    };
    kprintln!("FWCFG: found, signature \"QEMU\" verified via DMA");
    let fb_pa = frames::alloc_contiguous(W * H * 4 / frames::FRAME_SIZE)
        .expect("no contiguous framebuffer memory");
    if fwcfg::configure_ramfb(&fw, fb_pa, W as u32, H as u32).is_err() {
        kprintln!("FB_SKIP: no etc/ramfb file (run QEMU with -device ramfb)");
        frames::free(fb_pa, W * H * 4 / frames::FRAME_SIZE);
        return None;
    }
    let mut mode = [0u8; 16];
    let scene_mode = fw
        .find_file("opt/veil.mode")
        .and_then(|f| {
            let n = fw.read_file(f, &mut mode).ok()?;
            Some(&mode[..n] == b"m5scene")
        })
        .unwrap_or(false);
    let fb = unsafe { fb::Framebuffer::new(fb_pa as *mut u32, W, H, W * 4) };

    // Test scene. The screenshot checker asserts these exact colors at
    // known coordinates -- keep them in sync with scripts/verify_m5.py.
    fb.clear(0xff10_2040);
    fb.fill_rect(50, 50, 100, 100, 0xffe0_3030);
    fb.fill_rect(200, 50, 100, 100, 0xff30_c060);
    fb.fill_rect(350, 50, 100, 100, 0xff30_60e0);
    for i in 0..4isize {
        // window-frame border, 4px
        let (r, b) = (W as isize - 1 - i, H as isize - 1 - i);
        fb.draw_line(i, i, r, i, 0xffff_ffff);
        fb.draw_line(i, b, r, b, 0xffff_ffff);
        fb.draw_line(i, i, i, b, 0xffff_ffff);
        fb.draw_line(r, i, r, b, 0xffff_ffff);
    }
    fb.draw_line(50, 200, 450, 260, 0xffff_d040); // sloped lines
    fb.draw_line(50, 260, 450, 200, 0xffff_d040);
    for x in 0..W {
        // gradient strip: catches stride and byte-order mistakes visually
        let c = (x * 255 / (W - 1)) as u32;
        fb.fill_rect(x, 600, 1, 32, 0xff00_0000 | c << 16 | (c / 2) << 8);
    }
    fb.draw_string(50, 300, "VEIL OS", 0xffff_ffff, None);
    fb.draw_string(
        50,
        324,
        "M5: fw_cfg + ramfb framebuffer, lines, rects, and this bitmap font.",
        0xffc0_d0ff,
        None,
    );
    fb.draw_string(50, 348, "abcdefghijklmnopqrstuvwxyz 0123456789 !@#$%^&*()", 0xff90_a0c0, None);

    kprintln!("FB_OK: ramfb {W}x{H} XRGB8888 at pa {fb_pa:#x}");
    kprintln!("M5_OK");
    Some((fb, scene_mode))
}

/// M2: deliberate traps survive, GIC + generic timer tick.
fn milestone2(fdt: &dtb::Fdt) {
    let (addr_cells, size_cells) = fdt.root_cells();

    let gic_node = fdt
        .find_compatible("arm,cortex-a15-gic")
        .expect("GICv2 node not in DTB");
    let reg = fdt.prop(gic_node, "reg").expect("GIC node has no reg");
    let gicd = dtb::cells(reg, 0, addr_cells) as usize;
    let gicc = dtb::cells(reg, (addr_cells + size_cells) * 4, addr_cells) as usize;
    kprintln!("DTB: GICv2 distributor @ {gicd:#x}, cpu interface @ {gicc:#x}");

    let timer_node = fdt
        .find_compatible("arm,armv8-timer")
        .expect("armv8-timer node not in DTB");
    let irqs = fdt.prop(timer_node, "interrupts").expect("timer has no interrupts");
    // Four 3-cell entries (type, number, flags): sec-phys, phys, virt, hyp.
    // We drive the non-secure physical timer, entry 1. PPI n => INTID 16+n.
    let irq_type = dtb::cells(irqs, 12, 1);
    let ppi = dtb::cells(irqs, 16, 1) as u32;
    assert!(irq_type == 1, "physical timer interrupt is not a PPI?");
    let timer_intid = 16 + ppi;
    kprintln!(
        "DTB: physical timer PPI {ppi} -> INTID {timer_intid}, freq {} Hz",
        timer::frequency()
    );

    exceptions::install();

    unsafe { core::arch::asm!("svc #42") };
    kprintln!("TRAP_OK: survived deliberate svc");

    unsafe { core::arch::asm!("brk #7") };
    kprintln!("TRAP_OK: survived deliberate brk");

    gic::init(gicd, gicc);
    gic::register_handler(timer_intid, |_| {
        timer::on_tick();
        net::on_tick(); // TCP retransmission / TIME_WAIT housekeeping
        scheduler::tick();
    });
    gic::enable(timer_intid);
    timer::start(timer_intid, 10); // 100 ms tick
    exceptions::enable_irqs();

    while timer::ticks() < 10 {
        unsafe { core::arch::asm!("wfi") };
    }
    timer::stop();
    kprintln!("TIMER_OK: observed {} timer ticks", timer::ticks());
    kprintln!("M2_OK");
}

/// M3: frame allocator from DTB RAM size, MMU on, fresh mapping readable.
fn milestone3(fdt: &dtb::Fdt, dtb_pa: usize) -> paging::Mapper {
    let (addr_cells, size_cells) = fdt.root_cells();

    let mem_node = fdt.find_device_type("memory").expect("no /memory in DTB");
    let reg = fdt.prop(mem_node, "reg").expect("memory node has no reg");
    let ram_base = dtb::cells(reg, 0, addr_cells) as usize;
    let ram_size = dtb::cells(reg, addr_cells * 4, size_cells) as usize;
    kprintln!("DTB: RAM {} MiB at {ram_base:#x}", ram_size >> 20);

    let kernel_start = &raw const __kernel_start as usize;
    let kernel_end = &raw const __kernel_end as usize;
    frames::init(
        ram_base,
        ram_size,
        &[
            (dtb_pa, dtb_pa + fdt.total_size()),
            (kernel_start, kernel_end),
        ],
    );
    kprintln!(
        "FRAMES: {} free frames ({} MiB), kernel {kernel_start:#x}..{kernel_end:#x} reserved",
        frames::free_frames(),
        frames::free_frames() * frames::FRAME_SIZE >> 20
    );

    let mut mapper = paging::Mapper::new();
    mapper.identity_map_machine(ram_base, ram_size);
    mapper.enable();
    kprintln!("MMU_ON: identity map active, caches enabled");

    // The proof: a brand-new VA -> fresh frame mapping, written through the
    // VA, read back through both the VA and its physical alias.
    const TEST_VA: usize = 0x10_0000_0000; // far outside any identity block
    const SENTINEL: u64 = 0xCAFE_F00D_DEAD_BEEF;
    let frame = frames::alloc().expect("no frame for paging test");
    mapper.map_page(TEST_VA, frame, false);
    let (via_va, via_pa) = unsafe {
        write_volatile(TEST_VA as *mut u64, SENTINEL);
        (
            read_volatile(TEST_VA as *const u64),
            read_volatile(frame as *const u64),
        )
    };
    assert_eq!(via_va, SENTINEL);
    assert_eq!(via_pa, SENTINEL, "PA alias disagrees with VA");
    kprintln!(
        "PAGING_OK: wrote {SENTINEL:#x} via va {TEST_VA:#x}, read back {via_va:#x} (pa {frame:#x} alias matches)"
    );
    kprintln!("M3_OK");
    mapper
}

/// M4: global allocator under an interleaved alloc/free stress, no leaks.
fn milestone4() {
    use alloc::{collections::BTreeMap, string::String, vec::Vec};
    use core::fmt::Write;

    const HEAP_FRAMES: usize = 4096; // 16 MiB
    let heap_pa = frames::alloc_contiguous(HEAP_FRAMES).expect("no contiguous heap region");
    heap::init(heap_pa, HEAP_FRAMES * frames::FRAME_SIZE);

    let before = heap::free_bytes();
    let mut allocs: u64 = 0;
    {
        // Deterministic xorshift so failures reproduce exactly.
        let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };

        let mut slots: Vec<Option<Vec<u8>>> = Vec::new();
        slots.resize_with(128, || None);
        for _ in 0..20_000 {
            let i = next() as usize % slots.len();
            match slots[i].take() {
                Some(v) => {
                    // Every byte must still hold the fill pattern, or some
                    // other allocation trampled this one.
                    let pat = v[0];
                    assert!(v.iter().all(|&b| b == pat), "heap corruption detected");
                }
                None => {
                    let len = 1 + next() as usize % 2048;
                    let mut v = Vec::with_capacity(len);
                    v.resize(len, next() as u8);
                    slots[i] = Some(v);
                    allocs += 1;
                }
            }
        }

        let mut map = BTreeMap::new();
        for k in 0..1000u64 {
            map.insert(k, k * k);
        }
        for k in 0..1000u64 {
            assert_eq!(map[&k], k * k);
        }

        let mut s = String::new();
        for i in 0..200 {
            let _ = write!(s, "[{i}]");
        }
        assert!(s.ends_with("[199]"));
    } // everything drops here
    let after = heap::free_bytes();

    kprintln!(
        "HEAP_OK: {allocs} varied allocations interleaved with frees; free bytes before={before} after={after}"
    );
    assert_eq!(before, after, "heap leaked");
    kprintln!("M4_OK");
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("KERNEL PANIC: {info}");
    semihosting::exit(1)
}
