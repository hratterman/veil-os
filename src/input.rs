//! virtio-input driver (device ID 18): keyboard + tablet over virtio-mmio.
//!
//! Each device gets an eventq of device-writable 8-byte buffers (evdev
//! events: type u16, code u16, value u32, little-endian). The IRQ handler
//! drains used buffers into a lock-free SPSC ring; the desktop's main loop
//! consumes from it. The statusq (LEDs) is left unconfigured.

use crate::{dtb, frames, gic, kprintln, virtio};
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

const VIRTIO_ID_INPUT: u32 = 18;
const QUEUE_LEN: u16 = 64;

// virtio-input config selectors.
const CFG_ID_NAME: u8 = 0x01;
const CFG_ABS_INFO: u8 = 0x12;

struct Device {
    mmio: virtio::Mmio,
    intid: u32,
    queue: virtio::Queue,
    buffers: usize, // PA of QUEUE_LEN 8-byte event buffers
}

const MAX_DEVICES: usize = 4;
static mut DEVICES: [Option<Device>; MAX_DEVICES] = [const { None }; MAX_DEVICES];

// SPSC ring: IRQ handler produces, main loop consumes.
const RING: usize = 512;
static EVENTS: [AtomicU64; RING] = [const { AtomicU64::new(0) }; RING];
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

// Tablet axis ranges from device config (defaults match QEMU).
static ABS_MAX_X: AtomicU32 = AtomicU32::new(32767);
static ABS_MAX_Y: AtomicU32 = AtomicU32::new(32767);

pub fn abs_max() -> (u32, u32) {
    (ABS_MAX_X.load(Ordering::Relaxed), ABS_MAX_Y.load(Ordering::Relaxed))
}

/// Pop one raw evdev event: (type, code, value).
pub fn pop() -> Option<(u16, u16, u32)> {
    let tail = TAIL.load(Ordering::Relaxed);
    if tail == HEAD.load(Ordering::Acquire) {
        return None;
    }
    let ev = EVENTS[tail % RING].load(Ordering::Relaxed);
    TAIL.store(tail + 1, Ordering::Release);
    Some((ev as u16, (ev >> 16) as u16, (ev >> 32) as u32))
}

fn push(ev_type: u16, code: u16, value: u32) {
    let head = HEAD.load(Ordering::Relaxed);
    if head - TAIL.load(Ordering::Acquire) >= RING {
        return; // full: drop newest
    }
    let packed = ev_type as u64 | (code as u64) << 16 | (value as u64) << 32;
    EVENTS[head % RING].store(packed, Ordering::Relaxed);
    HEAD.store(head + 1, Ordering::Release);
}

/// Read a config-space string (e.g. the device name).
fn config_name(mmio: &virtio::Mmio, buf: &mut [u8]) -> usize {
    mmio.config_write_u8(0, CFG_ID_NAME);
    mmio.config_write_u8(1, 0);
    let size = (mmio.config_read_u8(2) as usize).min(buf.len());
    for (i, b) in buf.iter_mut().take(size).enumerate() {
        *b = mmio.config_read_u8(8 + i);
    }
    size
}

/// Axis range for ABS axis `axis`, if the device reports one.
fn config_abs_max(mmio: &virtio::Mmio, axis: u8) -> Option<u32> {
    mmio.config_write_u8(0, CFG_ABS_INFO);
    mmio.config_write_u8(1, axis);
    if mmio.config_read_u8(2) < 8 {
        return None;
    }
    let mut max = [0u8; 4];
    for (i, b) in max.iter_mut().enumerate() {
        *b = mmio.config_read_u8(8 + 4 + i); // payload = {min u32, max u32, ...} LE
    }
    Some(u32::from_le_bytes(max))
}

/// Probe every virtio-mmio slot in the DTB, bring up each input device,
/// hook its IRQ. Returns the number of input devices found.
pub fn init(fdt: &dtb::Fdt) -> usize {
    let (addr_cells, _) = fdt.root_cells();
    let mut count = 0;
    let mut node = fdt.find_compatible("virtio,mmio");
    while let Some(n) = node {
        if count == MAX_DEVICES {
            break;
        }
        let reg = fdt.prop(n, "reg").expect("virtio node without reg");
        let base = dtb::cells(reg, 0, addr_cells) as usize;
        let mmio = virtio::Mmio { base };
        if mmio.probe() == Some(VIRTIO_ID_INPUT) {
            // interrupts = <type num flags>; SPI => INTID = 32 + num.
            let irq = fdt.prop(n, "interrupts").expect("virtio node without irq");
            assert!(dtb::cells(irq, 0, 1) == 0, "virtio irq is not an SPI?");
            let intid = 32 + dtb::cells(irq, 4, 1) as u32;
            init_device(mmio, intid, count);
            count += 1;
        }
        node = fdt.find_compatible_after("virtio,mmio", n);
    }
    count
}

fn init_device(mmio: virtio::Mmio, intid: u32, slot: usize) {
    mmio.init(0).expect("virtio-input feature negotiation failed");

    let mut name = [0u8; 64];
    let name_len = config_name(&mmio, &mut name);
    let name_str = core::str::from_utf8(&name[..name_len]).unwrap_or("?");

    if let Some(max_x) = config_abs_max(&mmio, 0) {
        ABS_MAX_X.store(max_x, Ordering::Relaxed);
        if let Some(max_y) = config_abs_max(&mmio, 1) {
            ABS_MAX_Y.store(max_y, Ordering::Relaxed);
        }
    }

    let ring_mem = frames::alloc_zeroed().expect("no frame for virtqueue");
    assert!(virtio::Queue::bytes_needed(QUEUE_LEN as usize) <= frames::FRAME_SIZE);
    let buffers = frames::alloc_zeroed().expect("no frame for event buffers");

    let mut queue = virtio::Queue::new(QUEUE_LEN, ring_mem);
    for i in 0..QUEUE_LEN {
        queue.write_desc(i, (buffers + 8 * i as usize) as u64, 8, virtio::DESC_F_WRITE, 0);
        queue.push_avail(i);
    }
    mmio.setup_queue(0, &queue);
    mmio.driver_ok();
    mmio.notify(0);

    gic::register_handler(intid, on_irq);
    gic::set_edge(intid);
    gic::enable(intid);

    kprintln!("INPUT: \"{name_str}\" at {:#x}, INTID {intid}", mmio.base);
    unsafe {
        (*core::ptr::addr_of_mut!(DEVICES))[slot] =
            Some(Device { mmio, intid, queue, buffers });
    }
}

/// IRQ path: ack, drain used ring into the SPSC queue, repost buffers.
fn on_irq(intid: u32) {
    let devices = unsafe { &mut *core::ptr::addr_of_mut!(DEVICES) };
    for dev in devices.iter_mut().flatten() {
        if dev.intid != intid {
            continue;
        }
        dev.mmio.irq_ack();
        let mut reposted = false;
        while let Some(id) = dev.queue.pop_used() {
            let buf = (dev.buffers + 8 * id as usize) as *const u8;
            let raw = unsafe { core::ptr::read_volatile(buf as *const u64) };
            // evdev event, LE: type u16 | code u16 | value u32
            push(raw as u16, (raw >> 16) as u16, (raw >> 32) as u32);
            dev.queue.push_avail(id);
            reposted = true;
        }
        if reposted {
            dev.mmio.notify(0);
        }
    }
}
