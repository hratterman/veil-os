//! M17: VideoCore mailbox property interface (BCM2711).
//!
//! The ARM core talks to the GPU firmware through mailbox 8 (the property
//! channel): write the physical address of a 16-byte-aligned tag buffer,
//! the firmware overwrites it in place with responses. This is how a bare
//! Pi kernel learns its ARM-visible RAM split and gets a framebuffer
//! (there is no ramfb/fw_cfg on metal).
//!
//! Every call here happens before the MMU/caches are enabled, so the
//! buffer is coherent with the GPU's view without any cache maintenance.

use core::ptr::{read_volatile, write_volatile};

const MBOX_BASE: usize = 0xFE00_B880;
const MBOX_READ: usize = MBOX_BASE + 0x00;
const MBOX_STATUS: usize = MBOX_BASE + 0x18; // mailbox 0 (VC -> ARM)
const MBOX_WRITE: usize = MBOX_BASE + 0x20; // mailbox 1 (ARM -> VC)
const MBOX_WSTATUS: usize = MBOX_BASE + 0x38;

const STATUS_FULL: u32 = 1 << 31;
const STATUS_EMPTY: u32 = 1 << 30;
const CHANNEL_PROPS: u32 = 8;

const REQUEST: u32 = 0;
const RESPONSE_OK: u32 = 0x8000_0000;

// Property tags.
const TAG_GET_ARM_MEMORY: u32 = 0x0001_0005;
const TAG_GET_VC_MEMORY: u32 = 0x0001_0006;
const TAG_ALLOC_FB: u32 = 0x0004_0001;
const TAG_GET_PITCH: u32 = 0x0004_0008;
const TAG_SET_PHYS_WH: u32 = 0x0004_8003;
const TAG_SET_VIRT_WH: u32 = 0x0004_8004;
const TAG_SET_DEPTH: u32 = 0x0004_8005;
const TAG_SET_PIXEL_ORDER: u32 = 0x0004_8006;
const TAG_SET_VIRT_OFFSET: u32 = 0x0004_8009;
const TAG_END: u32 = 0;

#[repr(C, align(16))]
struct Buffer([u32; 64]);

static mut BUFFER: Buffer = Buffer([0; 64]);

/// Run one property call: `words` is the tag area (between the header and
/// the end tag). Returns the response tag area, or None if the firmware
/// rejected the buffer.
fn call(words: &[u32]) -> Option<[u32; 61]> {
    let buf = &raw mut BUFFER;
    unsafe {
        write_volatile(&raw mut (*buf).0[0], (words.len() as u32 + 3) * 4);
        write_volatile(&raw mut (*buf).0[1], REQUEST);
        for (i, &w) in words.iter().enumerate() {
            write_volatile(&raw mut (*buf).0[2 + i], w);
        }
        write_volatile(&raw mut (*buf).0[2 + words.len()], TAG_END);

        while read_volatile(MBOX_WSTATUS as *const u32) & STATUS_FULL != 0 {}
        let addr = buf as usize as u32;
        write_volatile(MBOX_WRITE as *mut u32, (addr & !0xf) | CHANNEL_PROPS);
        loop {
            while read_volatile(MBOX_STATUS as *const u32) & STATUS_EMPTY != 0 {}
            let resp = read_volatile(MBOX_READ as *const u32);
            if resp & 0xf == CHANNEL_PROPS && resp & !0xf == addr & !0xf {
                break;
            }
        }
        if read_volatile(&raw const (*buf).0[1]) != RESPONSE_OK {
            return None;
        }
        let mut out = [0u32; 61];
        for (i, w) in out.iter_mut().enumerate() {
            *w = read_volatile(&raw const (*buf).0[2 + i]);
        }
        Some(out)
    }
}

/// (base, size) of the ARM-visible RAM block, straight from the firmware.
pub fn arm_memory() -> Option<(usize, usize)> {
    let r = call(&[TAG_GET_ARM_MEMORY, 8, 0, 0, 0])?;
    Some((r[3] as usize, r[4] as usize))
}

/// (base, size) of the VideoCore-owned RAM block.
pub fn vc_memory() -> Option<(usize, usize)> {
    let r = call(&[TAG_GET_VC_MEMORY, 8, 0, 0, 0])?;
    Some((r[3] as usize, r[4] as usize))
}

pub struct FbInfo {
    pub addr: usize, // ARM physical address (bus alias masked off)
    pub size: usize,
    pub pitch: usize,
    pub width: usize,
    pub height: usize,
}

/// Request a 32bpp framebuffer from the firmware. Pixel order 0 (BGR) so
/// the in-memory byte layout of our 0xAARRGGBB little-endian u32 writes
/// (B, G, R, A ascending) scans out with red as red.
pub fn alloc_framebuffer(width: usize, height: usize) -> Option<FbInfo> {
    let r = call(&[
        TAG_SET_PHYS_WH, 8, 0, width as u32, height as u32,
        TAG_SET_VIRT_WH, 8, 0, width as u32, height as u32,
        TAG_SET_VIRT_OFFSET, 8, 0, 0, 0,
        TAG_SET_DEPTH, 4, 0, 32,
        TAG_SET_PIXEL_ORDER, 4, 0, 0,
        TAG_ALLOC_FB, 8, 0, 4096, 0,
        TAG_GET_PITCH, 4, 0, 0,
    ])?;
    // Response layout mirrors the request; values land in the same slots:
    // phys w/h at 3/4, depth at 18, alloc addr/size at 26/27, pitch at 31.
    let (w, h) = (r[3] as usize, r[4] as usize);
    let depth = r[18];
    let addr = (r[26] & 0x3fff_ffff) as usize; // mask the VC bus alias
    let size = r[27] as usize;
    let pitch = r[31] as usize;
    if addr == 0 || depth != 32 {
        return None;
    }
    Some(FbInfo { addr, size, pitch, width: w, height: h })
}
