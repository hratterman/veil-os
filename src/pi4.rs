//! M17: the Raspberry Pi 4 (BCM2711) boot path.
//!
//! Same kernel, real silicon: the Pi firmware loads kernel8.img at
//! 0x8_0000 and jumps to it (header in pi4_head.s, link script
//! linker-pi4.ld). There is no fw_cfg, no ramfb and no virtio here —
//! RAM split and framebuffer come from the VideoCore mailbox, the UART
//! sits at the BCM2711 address, and the deliverable is the desktop
//! composited onto a real HDMI monitor.
//!
//! Deliberately minimal (the M17 pass is "boots and renders graphics"):
//! exceptions are installed for fault visibility but interrupts stay
//! masked; input/disk/network on metal are stretch goals, not this.

use crate::fb::Framebuffer;
use crate::wm::{App, PaintState, Wm};
use crate::{dtb, exceptions, frames, heap, kprintln, mbox, paging, uart};
use alloc::string::String;
use alloc::vec::Vec;

unsafe extern "C" {
    static __kernel_end: u8;
}

pub fn main(dtb_ptr: *const u8) -> ! {
    let serial = unsafe { uart::Pl011::new(uart::UART0_BASE) };
    serial.init();
    serial.put_str("BOOT_OK: veil kernel alive on BCM2711\n");

    exceptions::install();

    // The firmware, not a DTB, is the authority for the RAM split here.
    let (arm_base, arm_size) = mbox::arm_memory().expect("mailbox: get ARM memory failed");
    let (vc_base, vc_size) = mbox::vc_memory().unwrap_or((0, 0));
    kprintln!(
        "PI: ARM RAM {} MiB at {arm_base:#x}, VC RAM {} MiB at {vc_base:#x}",
        arm_size >> 20,
        vc_size >> 20
    );

    let fb = mbox::alloc_framebuffer(1024, 768).expect("mailbox: framebuffer alloc failed");
    kprintln!(
        "PI_FB: {}x{} 32bpp at {:#x}, pitch {} ({} KiB)",
        fb.width, fb.height, fb.addr, fb.pitch, fb.size >> 10
    );

    // Reserve everything from the spin tables at 0 through the kernel
    // image + boot stack, plus the DTB if the loader passed one.
    let kernel_end = &raw const __kernel_end as usize;
    let dtb_reserve = if dtb_ptr.is_null() {
        (0, 0)
    } else {
        match unsafe { dtb::Fdt::new(dtb_ptr) } {
            Some(fdt) => (dtb_ptr as usize, dtb_ptr as usize + fdt.total_size()),
            None => (0, 0),
        }
    };
    frames::init(arm_base, arm_size, &[(0, kernel_end), dtb_reserve]);
    kprintln!("FRAMES: {} free frames", frames::free_frames());

    // Identity map. RAM in 2 MiB normal blocks; the framebuffer (VC-owned,
    // typically above the ARM split) and everything else non-RAM in device
    // blocks so pixel writes go straight to the GPU's view. Peripherals
    // (0xFC00_0000+) get a device GiB at L1[3].
    let mut mapper = paging::Mapper::new();
    const TWO_M: usize = 2 << 20;
    let arm_end = arm_base + arm_size;
    let fb_start = fb.addr & !(TWO_M - 1);
    let fb_end = (fb.addr + fb.size + TWO_M - 1) & !(TWO_M - 1);
    let map_end = arm_end.max(fb_end).min(0xC000_0000);
    let mut pa = 0;
    while pa < map_end {
        let in_fb = pa + TWO_M > fb_start && pa < fb_end;
        let in_ram = pa >= arm_base && pa + TWO_M <= arm_end;
        mapper.map_block_2m(pa, pa, in_fb || !in_ram);
        pa += TWO_M;
    }
    mapper.map_block_1g(0xC000_0000, 0xC000_0000, true);
    mapper.enable();
    kprintln!("MMU_ON: identity map active, framebuffer mapped as device");

    const HEAP_FRAMES: usize = 4096; // 16 MiB
    let heap_pa = frames::alloc_contiguous(HEAP_FRAMES).expect("no contiguous heap region");
    heap::init(heap_pa, HEAP_FRAMES * frames::FRAME_SIZE);
    kprintln!("HEAP_OK: {} MiB", (HEAP_FRAMES * frames::FRAME_SIZE) >> 20);

    // The same desktop the QEMU build composes, on a real monitor.
    let screen = unsafe { Framebuffer::new(fb.addr as *mut u32, fb.width, fb.height, fb.pitch) };
    let mut wm = Wm::new(screen, (32767, 32767));
    wm.add_window(
        "shell",
        40,
        430,
        420,
        280,
        App::Shell { input: String::new(), lines: Vec::new() },
    );
    wm.add_window("alpha", 60, 60, 380, 200, App::Echo { text: String::from("veil on metal") });
    wm.add_window("beta", 260, 160, 380, 200, App::Static);
    wm.add_window("paint", 480, 330, 480, 380, App::Paint(PaintState::new()));
    wm.compose();
    kprintln!(
        "PI_FB_OK: desktop ({} windows) composited to the mailbox framebuffer",
        wm.windows.len()
    );
    kprintln!("M17_OK");

    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}
