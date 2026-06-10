//! Virtio over MMIO (modern, version 2) + split virtqueues. This is the
//! machinery M6 needs for input and that M10/M12 will reuse for block and
//! net. Run QEMU with `-global virtio-mmio.force-legacy=false`.

use core::ptr::{read_volatile, write_volatile};

// MMIO register offsets (virtio spec 4.2.2).
const MAGIC: usize = 0x000; // "virt"
const VERSION: usize = 0x004; // 2 = modern
const DEVICE_ID: usize = 0x008; // 0 = empty slot, 18 = input
const DEV_FEATURES: usize = 0x010;
const DEV_FEATURES_SEL: usize = 0x014;
const DRV_FEATURES: usize = 0x020;
const DRV_FEATURES_SEL: usize = 0x024;
const QUEUE_SEL: usize = 0x030;
const QUEUE_NUM_MAX: usize = 0x034;
const QUEUE_NUM: usize = 0x038;
const QUEUE_READY: usize = 0x044;
const QUEUE_NOTIFY: usize = 0x050;
const INT_STATUS: usize = 0x060;
const INT_ACK: usize = 0x064;
const STATUS: usize = 0x070;
const QUEUE_DESC_LO: usize = 0x080;
const QUEUE_DESC_HI: usize = 0x084;
const QUEUE_DRIVER_LO: usize = 0x090; // avail ring
const QUEUE_DRIVER_HI: usize = 0x094;
const QUEUE_DEVICE_LO: usize = 0x0a0; // used ring
const QUEUE_DEVICE_HI: usize = 0x0a4;
const CONFIG: usize = 0x100;

const ST_ACKNOWLEDGE: u32 = 1;
const ST_DRIVER: u32 = 2;
const ST_DRIVER_OK: u32 = 4;
const ST_FEATURES_OK: u32 = 8;

pub const DESC_F_NEXT: u16 = 1; // chained to .next
pub const DESC_F_WRITE: u16 = 2; // device-writable buffer

fn dmb() {
    unsafe { core::arch::asm!("dmb ish", options(nostack)) };
}

#[derive(Clone, Copy)]
pub struct Mmio {
    pub base: usize,
}

impl Mmio {
    fn read(&self, off: usize) -> u32 {
        unsafe { read_volatile((self.base + off) as *const u32) }
    }

    fn write(&self, off: usize, val: u32) {
        unsafe { write_volatile((self.base + off) as *mut u32, val) }
    }

    /// Returns the device ID if this slot holds a live modern device.
    pub fn probe(&self) -> Option<u32> {
        if self.read(MAGIC) != 0x7472_6976 || self.read(VERSION) != 2 {
            return None;
        }
        match self.read(DEVICE_ID) {
            0 => None,
            id => Some(id),
        }
    }

    /// Reset, acknowledge, and negotiate features. We accept only
    /// VIRTIO_F_VERSION_1 (bit 32) plus whatever `extra_lo` bits the caller
    /// wants from the low feature word.
    pub fn init(&self, extra_lo: u32) -> Result<(), ()> {
        self.write(STATUS, 0);
        self.write(STATUS, ST_ACKNOWLEDGE);
        self.write(STATUS, ST_ACKNOWLEDGE | ST_DRIVER);

        self.write(DEV_FEATURES_SEL, 1);
        if self.read(DEV_FEATURES) & 1 == 0 {
            return Err(()); // device doesn't offer VERSION_1?
        }
        self.write(DEV_FEATURES_SEL, 0);
        let lo = self.read(DEV_FEATURES) & extra_lo;
        self.write(DRV_FEATURES_SEL, 0);
        self.write(DRV_FEATURES, lo);
        self.write(DRV_FEATURES_SEL, 1);
        self.write(DRV_FEATURES, 1); // VERSION_1

        self.write(STATUS, ST_ACKNOWLEDGE | ST_DRIVER | ST_FEATURES_OK);
        if self.read(STATUS) & ST_FEATURES_OK == 0 {
            return Err(());
        }
        Ok(())
    }

    /// Install a queue's rings and mark it ready.
    pub fn setup_queue(&self, idx: u32, q: &Queue) {
        self.write(QUEUE_SEL, idx);
        assert!(self.read(QUEUE_NUM_MAX) >= q.n as u32, "queue too big");
        self.write(QUEUE_NUM, q.n as u32);
        self.write(QUEUE_DESC_LO, q.desc as u32);
        self.write(QUEUE_DESC_HI, (q.desc as u64 >> 32) as u32);
        self.write(QUEUE_DRIVER_LO, q.avail as u32);
        self.write(QUEUE_DRIVER_HI, (q.avail as u64 >> 32) as u32);
        self.write(QUEUE_DEVICE_LO, q.used as u32);
        self.write(QUEUE_DEVICE_HI, (q.used as u64 >> 32) as u32);
        self.write(QUEUE_READY, 1);
    }

    pub fn driver_ok(&self) {
        self.write(STATUS, ST_ACKNOWLEDGE | ST_DRIVER | ST_FEATURES_OK | ST_DRIVER_OK);
    }

    pub fn notify(&self, queue: u32) {
        dmb();
        self.write(QUEUE_NOTIFY, queue);
    }

    /// Read-and-acknowledge pending interrupt causes.
    pub fn irq_ack(&self) -> u32 {
        let status = self.read(INT_STATUS);
        if status != 0 {
            self.write(INT_ACK, status);
        }
        status
    }

    pub fn config_read_u8(&self, off: usize) -> u8 {
        unsafe { read_volatile((self.base + CONFIG + off) as *const u8) }
    }

    pub fn config_write_u8(&self, off: usize, val: u8) {
        unsafe { write_volatile((self.base + CONFIG + off) as *mut u8, val) }
    }
}

/// A split virtqueue over one identity-mapped contiguous memory block:
/// descriptor table, then avail ring, then (4-aligned) used ring.
pub struct Queue {
    pub n: u16,
    desc: usize,
    avail: usize,
    used: usize,
    avail_idx: u16,
    last_used: u16,
}

impl Queue {
    pub fn bytes_needed(n: usize) -> usize {
        let after_avail = 16 * n + 4 + 2 * n;
        (after_avail + 3 & !3) + 4 + 8 * n
    }

    /// `mem` must be zeroed, at least `bytes_needed(n)`, 16-byte aligned.
    pub fn new(n: u16, mem: usize) -> Queue {
        let desc = mem;
        let avail = mem + 16 * n as usize;
        let used = (avail + 4 + 2 * n as usize + 3) & !3;
        Queue { n, desc, avail, used, avail_idx: 0, last_used: 0 }
    }

    pub fn write_desc(&self, i: u16, addr: u64, len: u32, flags: u16, next: u16) {
        let d = (self.desc + 16 * i as usize) as *mut u64;
        unsafe {
            write_volatile(d, addr);
            // len (u32) | flags (u16) | next (u16)
            write_volatile(
                d.add(1),
                len as u64 | (flags as u64) << 32 | (next as u64) << 48,
            );
        }
    }

    /// Publish descriptor `id` to the device.
    pub fn push_avail(&mut self, id: u16) {
        let slot = (self.avail + 4 + 2 * (self.avail_idx % self.n) as usize) as *mut u16;
        unsafe {
            write_volatile(slot, id);
            dmb();
            self.avail_idx = self.avail_idx.wrapping_add(1);
            write_volatile((self.avail + 2) as *mut u16, self.avail_idx);
        }
    }

    /// Pop one completion: the descriptor id the device finished with.
    pub fn pop_used(&mut self) -> Option<u16> {
        self.pop_used_len().map(|(id, _)| id)
    }

    /// Pop one completion with the byte count the device wrote (the used
    /// ring's `len` field) — virtio-net RX needs it to size the frame.
    pub fn pop_used_len(&mut self) -> Option<(u16, u32)> {
        let device_idx = unsafe { read_volatile((self.used + 2) as *const u16) };
        if device_idx == self.last_used {
            return None;
        }
        dmb();
        let elem = (self.used + 4 + 8 * (self.last_used % self.n) as usize) as *const u32;
        let id = unsafe { read_volatile(elem) } as u16;
        let len = unsafe { read_volatile(elem.add(1)) };
        self.last_used = self.last_used.wrapping_add(1);
        Some((id, len))
    }
}
