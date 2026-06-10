//! virtio-blk driver (device ID 2) over virtio-mmio, synchronous/polled.
//! Each request is the classic 3-descriptor chain: header (driver-owned),
//! data, status byte (device-written). One request in flight at a time;
//! callers serialize through fs.rs critical sections.

use crate::{dtb, frames, kprintln, virtio};
use core::ptr::{read_volatile, write_volatile};

const VIRTIO_ID_BLOCK: u32 = 2;
pub const SECTOR: usize = 512;

const T_IN: u32 = 0; // device -> memory (read)
const T_OUT: u32 = 1; // memory -> device (write)

struct Blk {
    mmio: virtio::Mmio,
    queue: virtio::Queue,
    scratch: usize, // frame: request header @0, status byte @512
}

static mut BLK: Option<Blk> = None;

fn blk() -> Option<&'static mut Blk> {
    unsafe { (*core::ptr::addr_of_mut!(BLK)).as_mut() }
}

/// Probe the DTB's virtio slots for a block device. Returns capacity in
/// sectors if found.
pub fn init(fdt: &dtb::Fdt) -> Option<u64> {
    let (addr_cells, _) = fdt.root_cells();
    let mut node = fdt.find_compatible("virtio,mmio");
    while let Some(n) = node {
        let reg = fdt.prop(n, "reg")?;
        let base = dtb::cells(reg, 0, addr_cells) as usize;
        let mmio = virtio::Mmio { base };
        if mmio.probe() == Some(VIRTIO_ID_BLOCK) {
            mmio.init(0).ok()?;
            let ring_mem = frames::alloc_zeroed()?;
            let queue = virtio::Queue::new(64, ring_mem);
            mmio.setup_queue(0, &queue);
            mmio.driver_ok();
            // config: capacity u64 LE at offset 0
            let capacity = (0..8).fold(0u64, |acc, i| {
                acc | (mmio.config_read_u8(i) as u64) << (8 * i)
            });
            let scratch = frames::alloc_zeroed()?;
            unsafe {
                *core::ptr::addr_of_mut!(BLK) = Some(Blk { mmio, queue, scratch });
            }
            kprintln!("BLK: virtio-blk at {base:#x}, {capacity} sectors");
            return Some(capacity);
        }
        node = fdt.find_compatible_after("virtio,mmio", n);
    }
    None
}

pub fn available() -> bool {
    unsafe { (*core::ptr::addr_of!(BLK)).is_some() }
}

/// One synchronous transfer. `buf` must be identity-mapped (heap/stack).
fn request(write: bool, lba: u64, buf: *mut u8, sectors: usize) -> Result<(), ()> {
    let dev = blk().ok_or(())?;
    let hdr = dev.scratch as *mut u32;
    let status = (dev.scratch + SECTOR) as *mut u8;
    unsafe {
        write_volatile(hdr, if write { T_OUT } else { T_IN });
        write_volatile(hdr.add(1), 0);
        write_volatile((dev.scratch + 8) as *mut u64, lba);
        write_volatile(status, 0xff);
    }
    let data_flags = if write { virtio::DESC_F_NEXT } else { virtio::DESC_F_NEXT | virtio::DESC_F_WRITE };
    dev.queue.write_desc(0, dev.scratch as u64, 16, virtio::DESC_F_NEXT, 1);
    dev.queue.write_desc(1, buf as u64, (sectors * SECTOR) as u32, data_flags, 2);
    dev.queue.write_desc(2, (dev.scratch + SECTOR) as u64, 1, virtio::DESC_F_WRITE, 0);
    dev.queue.push_avail(0);
    dev.mmio.notify(0);

    let mut spins = 0u64;
    loop {
        if dev.queue.pop_used().is_some() {
            break;
        }
        spins += 1;
        assert!(spins < 1_000_000_000, "virtio-blk request hung");
    }
    dev.mmio.irq_ack(); // polled driver: clear the pending bit anyway
    match unsafe { read_volatile(status) } {
        0 => Ok(()),
        e => {
            kprintln!("BLK: request failed, status {e}");
            Err(())
        }
    }
}

pub fn read_sectors(lba: u64, sectors: usize, buf: &mut [u8]) -> Result<(), ()> {
    assert!(buf.len() >= sectors * SECTOR);
    request(false, lba, buf.as_mut_ptr(), sectors)
}

pub fn write_sectors(lba: u64, sectors: usize, buf: &[u8]) -> Result<(), ()> {
    assert!(buf.len() >= sectors * SECTOR);
    request(true, lba, buf.as_ptr() as *mut u8, sectors)
}
