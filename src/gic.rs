//! GICv2 driver (distributor + CPU interface). Base addresses come from the
//! DTB (spec §4 forbids guessing them); they're stashed in statics so the
//! IRQ handler can reach them.

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicUsize, Ordering};

// Distributor registers.
const GICD_CTLR: usize = 0x000;
const GICD_ISENABLER: usize = 0x100; // 1 bit per INTID, 32 per word
const GICD_IPRIORITYR: usize = 0x400; // 1 byte per INTID
const GICD_ITARGETSR: usize = 0x800; // 1 byte per INTID (SPIs only)
const GICD_ICFGR: usize = 0xc00; // 2 bits per INTID (level/edge)

// CPU interface registers.
const GICC_CTLR: usize = 0x000;
const GICC_PMR: usize = 0x004;
const GICC_IAR: usize = 0x00c;
const GICC_EOIR: usize = 0x010;

/// IAR value meaning "no pending interrupt".
pub const SPURIOUS: u32 = 1023;

static GICD_BASE: AtomicUsize = AtomicUsize::new(0);
static GICC_BASE: AtomicUsize = AtomicUsize::new(0);

fn dist_write(offset: usize, value: u32) {
    let base = GICD_BASE.load(Ordering::Relaxed);
    unsafe { write_volatile((base + offset) as *mut u32, value) }
}

fn cpu_write(offset: usize, value: u32) {
    let base = GICC_BASE.load(Ordering::Relaxed);
    unsafe { write_volatile((base + offset) as *mut u32, value) }
}

fn cpu_read(offset: usize) -> u32 {
    let base = GICC_BASE.load(Ordering::Relaxed);
    unsafe { read_volatile((base + offset) as *const u32) }
}

pub fn init(gicd: usize, gicc: usize) {
    GICD_BASE.store(gicd, Ordering::Relaxed);
    GICC_BASE.store(gicc, Ordering::Relaxed);

    dist_write(GICD_CTLR, 0); // quiesce while configuring
    cpu_write(GICC_PMR, 0xff); // pass all priorities
    dist_write(GICD_CTLR, 1); // forward interrupts to CPU interfaces
    cpu_write(GICC_CTLR, 1); // signal them to this core
}

fn dist_read(offset: usize) -> u32 {
    let base = GICD_BASE.load(Ordering::Relaxed);
    unsafe { read_volatile((base + offset) as *const u32) }
}

/// Enable one interrupt at the distributor with a mid-scale priority.
/// SPIs (intid >= 32) additionally need a CPU target (the registers are
/// banked/RO for PPIs).
pub fn enable(intid: u32) {
    let base = GICD_BASE.load(Ordering::Relaxed);
    let prio_byte = (base + GICD_IPRIORITYR + intid as usize) as *mut u8;
    unsafe { write_volatile(prio_byte, 0x80) };
    if intid >= 32 {
        let target_byte = (base + GICD_ITARGETSR + intid as usize) as *mut u8;
        unsafe { write_volatile(target_byte, 0x01) }; // CPU interface 0
    }
    dist_write(
        GICD_ISENABLER + 4 * (intid as usize / 32),
        1 << (intid % 32),
    );
}

/// Configure an SPI as edge-triggered (QEMU's virtio-mmio IRQs are edge,
/// per the DTB interrupt flags).
pub fn set_edge(intid: u32) {
    let reg = GICD_ICFGR + 4 * (intid as usize / 16);
    let shift = (intid as usize % 16) * 2;
    let val = dist_read(reg) | (0b10 << shift);
    dist_write(reg, val);
}

// --- IRQ dispatch registry -------------------------------------------------

const MAX_INTID: usize = 128;
static HANDLERS: [AtomicUsize; MAX_INTID] = [const { AtomicUsize::new(0) }; MAX_INTID];

pub fn register_handler(intid: u32, handler: fn(u32)) {
    HANDLERS[intid as usize].store(handler as usize, Ordering::Relaxed);
}

/// Invoke the registered handler; false if nobody claimed this INTID.
pub fn dispatch(intid: u32) -> bool {
    let ptr = HANDLERS
        .get(intid as usize)
        .map_or(0, |h| h.load(Ordering::Relaxed));
    if ptr == 0 {
        return false;
    }
    let handler: fn(u32) = unsafe { core::mem::transmute(ptr) };
    handler(intid);
    true
}

/// Read GICC_IAR, acknowledging the highest-priority pending interrupt.
/// Returns the raw IAR value; pass it unmodified to `end_of_interrupt`.
pub fn acknowledge() -> u32 {
    cpu_read(GICC_IAR)
}

pub fn end_of_interrupt(iar: u32) {
    cpu_write(GICC_EOIR, iar);
}
