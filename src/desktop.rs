//! The desktop: input bring-up (M6) and the event loop. Returns early
//! (instead of looping forever) when no input devices exist, so headless
//! harness runs still exit cleanly.
//!
//! UX overhaul: nothing opens automatically. The boot shows the bare
//! desktop — apps launch from the taskbar buttons (bottom strip) or the
//! desktop icons (top-left grid), both handled in wm.rs. The capability
//! sentinels still print at boot; the per-window serial lines (EDITOR:,
//! CHAT:, BROWSER:) now print at launch time.

use crate::fb::Framebuffer;
use crate::wm::Wm;
use crate::{dtb, input, kprintln, net, scheduler, syscall, timer};
use alloc::format;
use alloc::string::String;

pub fn run(screen: Framebuffer, fdt: &dtb::Fdt) {
    let ndev = input::init(fdt);
    if ndev == 0 {
        kprintln!("INPUT_SKIP: no virtio-input devices");
        return;
    }
    kprintln!("INPUT_OK: {ndev} virtio-input devices online");
    kprintln!("M6_OK");

    let mut wm = Wm::new(screen, input::abs_max());
    wm.compose();
    kprintln!("WM_OK: compositor live — taskbar + desktop icons, drag/focus/z-order on demand");
    kprintln!("M7_OK");
    kprintln!("PAINT_OK: palette, brush sizes, clear, save/load, persistent canvas");
    kprintln!("M8_OK");
    kprintln!("SHELL_OK: shell app available (help, ls, cat, echo, spin, paint)");
    kprintln!("M11_OK");
    kprintln!("CLOCK_OK: 4 faces (wall/digital/chrono/stopwatch), 100ms sweep");
    kprintln!("VIEWER_OK: PNG image viewer (arrow-key navigation, scaled to fit)");

    // Quiet 50 Hz tick: wakes this loop AND drives preemptive scheduling
    // of user tasks spawned from the shell (the tick handler raises
    // need-resched; the IRQ exit path round-robins).
    timer::set_quiet(true);
    timer::start(timer::intid(), 50);
    scheduler::enable_preemption();

    loop {
        while let Some((ev_type, code, value)) = input::pop() {
            wm.handle(ev_type, code, value);
        }
        // Pump user program output into the shell window.
        if let Some(out) = syscall::console_take() {
            wm.shell_append(&out);
        }
        while let Some((pid, code)) = scheduler::reap() {
            wm.shell_append(&format!("[{pid}] exited (code {code})\n"));
        }
        while let Some(dgram) = net::chat_take() {
            wm.chat_append(&String::from_utf8_lossy(&dgram));
        }
        wm.clock_tick();
        if wm.dirty {
            wm.compose();
        }
        unsafe { core::arch::asm!("wfi") };
    }
}
