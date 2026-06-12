//! M41 step 20: virtio-gpu driver (device ID 16) over virtio-mmio.
//!
//! The desktop renders through the GPU instead of legacy ramfb: we create a 2D
//! host resource, attach a guest backing buffer (the compositor draws into it),
//! set it as scanout 0, and on each frame issue TRANSFER_TO_HOST_2D + a
//! RESOURCE_FLUSH for the dirty rectangle — the host/GPU does the screen update
//! (and only the changed region during a drag), not a per-pixel CPU push.

use crate::{dtb, frames, kprintln, virtio};
use core::ptr::write_volatile;

const VIRTIO_ID_GPU: u32 = 16;

// virtio-gpu control commands.
const CMD_GET_DISPLAY_INFO: u32 = 0x0100;
const CMD_RESOURCE_CREATE_2D: u32 = 0x0101;
const CMD_RESOURCE_FLUSH: u32 = 0x0104;
const CMD_TRANSFER_TO_HOST_2D: u32 = 0x0105;
const CMD_RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const CMD_SET_SCANOUT: u32 = 0x0103;
const FORMAT_B8G8R8X8: u32 = 2; // matches our XRGB8888 little-endian pixels
const RESOURCE_ID: u32 = 1;

struct Gpu {
    mmio: virtio::Mmio,
    queue: virtio::Queue,
    scratch: usize, // command buffer (and response area)
    backing: usize, // the framebuffer the compositor draws into
    w: u32,
    h: u32,
}

static mut GPU: Option<Gpu> = None;

fn gpu() -> Option<&'static mut Gpu> {
    unsafe { (*core::ptr::addr_of_mut!(GPU)).as_mut() }
}

pub fn available() -> bool {
    unsafe { (*core::ptr::addr_of!(GPU)).is_some() }
}

/// (backing PA, width, height) of the GPU framebuffer, if initialized.
pub fn framebuffer() -> Option<(usize, usize, usize)> {
    gpu().map(|g| (g.backing, g.w as usize, g.h as usize))
}

/// Send a control command (cmd bytes already in `scratch`) of `cmd_len`, with a
/// `resp_len`-byte response area at `scratch + 2048`. Returns the response type.
fn submit(g: &mut Gpu, cmd_len: u32, resp_len: u32) -> u32 {
    let resp = g.scratch + 2048;
    unsafe {
        // zero the response header type
        write_volatile(resp as *mut u32, 0);
    }
    g.queue.write_desc(0, g.scratch as u64, cmd_len, virtio::DESC_F_NEXT, 1);
    g.queue.write_desc(1, resp as u64, resp_len, virtio::DESC_F_WRITE, 0);
    g.queue.push_avail(0);
    g.mmio.notify(0);
    let mut spins = 0u64;
    while g.queue.pop_used().is_none() {
        g.mmio.irq_ack();
        spins += 1;
        if spins > 200_000_000 {
            kprintln!("GPU: command timed out");
            break;
        }
    }
    unsafe { core::ptr::read_volatile(resp as *const u32) }
}

// Build the 24-byte control header at `p`.
unsafe fn hdr(p: *mut u8, ty: u32) {
    core::ptr::write_bytes(p, 0, 24);
    write_volatile(p as *mut u32, ty);
}
unsafe fn put32(p: *mut u8, off: usize, v: u32) {
    write_volatile(p.add(off) as *mut u32, v);
}
unsafe fn put64(p: *mut u8, off: usize, v: u64) {
    write_volatile(p.add(off) as *mut u64, v);
}
unsafe fn rect(p: *mut u8, off: usize, x: u32, y: u32, w: u32, h: u32) {
    put32(p, off, x);
    put32(p, off + 4, y);
    put32(p, off + 8, w);
    put32(p, off + 12, h);
}

/// Probe + initialize the virtio-gpu device. Returns true if a display is up.
pub fn init(fdt: &dtb::Fdt) -> bool {
    let (addr_cells, _) = fdt.root_cells();
    let mut node = fdt.find_compatible("virtio,mmio");
    while let Some(n) = node {
        let Some(reg) = fdt.prop(n, "reg") else { break };
        let base = dtb::cells(reg, 0, addr_cells) as usize;
        let mmio = virtio::Mmio { base };
        if mmio.probe() == Some(VIRTIO_ID_GPU) {
            if mmio.init(0).is_err() {
                return false;
            }
            let Some(ring) = frames::alloc_zeroed() else { return false };
            let queue = virtio::Queue::new(64, ring);
            mmio.setup_queue(0, &queue);
            mmio.driver_ok();
            let Some(scratch) = frames::alloc_zeroed() else { return false };
            let mut g = Gpu { mmio, queue, scratch, backing: 0, w: 1024, h: 768 };

            // 1) display info -> resolution (fall back to 1024x768).
            unsafe { hdr(g.scratch as *mut u8, CMD_GET_DISPLAY_INFO) };
            submit(&mut g, 24, 24 + 16 * 24);
            let r = g.scratch + 2048 + 24; // first display's rect
            let (dw, dh) = unsafe {
                (
                    core::ptr::read_volatile((r + 8) as *const u32),
                    core::ptr::read_volatile((r + 12) as *const u32),
                )
            };
            if (640..=4096).contains(&dw) && (480..=4096).contains(&dh) {
                g.w = dw;
                g.h = dh;
            }
            // clamp to our supported max and a sane buffer size
            g.w = g.w.min(1024);
            g.h = g.h.min(768);

            let fb_bytes = g.w as usize * g.h as usize * 4;
            let pages = fb_bytes.div_ceil(frames::FRAME_SIZE);
            let Some(backing) = frames::alloc_contiguous(pages) else { return false };
            g.backing = backing;

            // 2) create a 2D resource.
            unsafe {
                let p = g.scratch as *mut u8;
                hdr(p, CMD_RESOURCE_CREATE_2D);
                put32(p, 24, RESOURCE_ID);
                put32(p, 28, FORMAT_B8G8R8X8);
                put32(p, 32, g.w);
                put32(p, 36, g.h);
            }
            submit(&mut g, 40, 24);

            // 3) attach the backing buffer.
            unsafe {
                let p = g.scratch as *mut u8;
                hdr(p, CMD_RESOURCE_ATTACH_BACKING);
                put32(p, 24, RESOURCE_ID);
                put32(p, 28, 1); // nr_entries
                put64(p, 32, backing as u64); // mem entry addr
                put32(p, 40, fb_bytes as u32); // length
                put32(p, 44, 0); // padding
            }
            submit(&mut g, 48, 24);

            // 4) set scanout 0 to this resource.
            unsafe {
                let p = g.scratch as *mut u8;
                hdr(p, CMD_SET_SCANOUT);
                rect(p, 24, 0, 0, g.w, g.h);
                put32(p, 40, 0); // scanout_id
                put32(p, 44, RESOURCE_ID);
            }
            submit(&mut g, 48, 24);

            kprintln!("GPU: virtio-gpu at {base:#x}, {}x{} scanout via resource {RESOURCE_ID}", g.w, g.h);
            kprintln!("GPU_OK: desktop renders through virtio-gpu (host-side blits)");
            let (w, h) = (g.w, g.h);
            unsafe { *core::ptr::addr_of_mut!(GPU) = Some(g) };
            let _ = (w, h);
            return true;
        }
        node = fdt.find_compatible_after("virtio,mmio", n);
    }
    false
}

/// Push the dirty rectangle `(x, y, w, h)` to the display: transfer the region
/// from the backing buffer to the host resource, then flush it to the screen.
pub fn flush_rect(x: u32, y: u32, w: u32, h: u32) {
    let Some(g) = gpu() else { return };
    let (gw, gh) = (g.w, g.h);
    let (x, y) = (x.min(gw), y.min(gh));
    let w = w.min(gw - x);
    let h = h.min(gh - y);
    if w == 0 || h == 0 {
        return;
    }
    let offset = (y as u64 * gw as u64 + x as u64) * 4;
    unsafe {
        let p = g.scratch as *mut u8;
        hdr(p, CMD_TRANSFER_TO_HOST_2D);
        rect(p, 24, x, y, w, h);
        put64(p, 40, offset);
        put32(p, 48, RESOURCE_ID);
        put32(p, 52, 0);
    }
    submit(g, 56, 24);
    unsafe {
        let p = g.scratch as *mut u8;
        hdr(p, CMD_RESOURCE_FLUSH);
        rect(p, 24, x, y, w, h);
        put32(p, 40, RESOURCE_ID);
        put32(p, 44, 0);
    }
    submit(g, 48, 24);
}

/// Present the whole frame (called after the compositor flip).
pub fn present() {
    if let Some((_, w, h)) = framebuffer() {
        flush_rect(0, 0, w as u32, h as u32);
    }
}
