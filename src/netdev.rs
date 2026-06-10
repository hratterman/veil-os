//! virtio-net driver (device ID 1) over virtio-mmio: raw Ethernet frames.
//!
//! Queue 0 is RX (device-writable buffers, drained from the IRQ handler
//! into net::on_frame), queue 1 is TX (one polled in-flight send, like the
//! blk driver). With VIRTIO_F_VERSION_1 every frame is prefixed by a
//! 12-byte virtio_net_hdr; we run with no offloads, so ours is all zeros
//! and we skip the device's on receive.

use crate::{dtb, frames, gic, kprintln, net, virtio};

const VIRTIO_ID_NET: u32 = 1;
const F_MAC: u32 = 1 << 5; // device exposes its MAC in config space

const HDR_LEN: usize = 12; // virtio_net_hdr with num_buffers (VERSION_1)
const BUF_LEN: usize = 2048; // hdr + max ethernet frame, power of two
const RX_BUFS: u16 = 32;
pub const MAX_FRAME: usize = 1514;

struct Net {
    mmio: virtio::Mmio,
    intid: u32,
    rx: virtio::Queue,
    tx: virtio::Queue,
    rx_bufs: usize, // PA of RX_BUFS * BUF_LEN
    tx_buf: usize,  // PA of one BUF_LEN slot
    mac: [u8; 6],
}

static mut NET: Option<Net> = None;

fn net() -> Option<&'static mut Net> {
    unsafe { (*core::ptr::addr_of_mut!(NET)).as_mut() }
}

pub fn mac() -> [u8; 6] {
    net().map_or([0; 6], |n| n.mac)
}

pub fn available() -> bool {
    unsafe { (*core::ptr::addr_of!(NET)).is_some() }
}

/// Probe the DTB's virtio slots for a network device; bring up RX/TX
/// queues and hook the IRQ. Returns the MAC if found.
pub fn init(fdt: &dtb::Fdt) -> Option<[u8; 6]> {
    let (addr_cells, _) = fdt.root_cells();
    let mut node = fdt.find_compatible("virtio,mmio");
    while let Some(n) = node {
        let reg = fdt.prop(n, "reg")?;
        let base = dtb::cells(reg, 0, addr_cells) as usize;
        let mmio = virtio::Mmio { base };
        if mmio.probe() == Some(VIRTIO_ID_NET) {
            let irq = fdt.prop(n, "interrupts").expect("virtio node without irq");
            assert!(dtb::cells(irq, 0, 1) == 0, "virtio irq is not an SPI?");
            let intid = 32 + dtb::cells(irq, 4, 1) as u32;
            return Some(init_device(mmio, intid));
        }
        node = fdt.find_compatible_after("virtio,mmio", n);
    }
    None
}

fn init_device(mmio: virtio::Mmio, intid: u32) -> [u8; 6] {
    mmio.init(F_MAC).expect("virtio-net feature negotiation failed");
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = mmio.config_read_u8(i);
    }

    let rx_ring = frames::alloc_zeroed().expect("no frame for net rx ring");
    let tx_ring = frames::alloc_zeroed().expect("no frame for net tx ring");
    assert!(virtio::Queue::bytes_needed(RX_BUFS as usize) <= frames::FRAME_SIZE);
    let rx_bufs = frames::alloc_contiguous(RX_BUFS as usize * BUF_LEN / frames::FRAME_SIZE)
        .expect("no contiguous net rx buffers");
    let tx_buf = frames::alloc_zeroed().expect("no frame for net tx buffer");

    let mut rx = virtio::Queue::new(RX_BUFS, rx_ring);
    for i in 0..RX_BUFS {
        rx.write_desc(
            i,
            (rx_bufs + BUF_LEN * i as usize) as u64,
            BUF_LEN as u32,
            virtio::DESC_F_WRITE,
            0,
        );
        rx.push_avail(i);
    }
    let tx = virtio::Queue::new(8, tx_ring);

    mmio.setup_queue(0, &rx);
    mmio.setup_queue(1, &tx);
    mmio.driver_ok();
    mmio.notify(0);

    gic::register_handler(intid, on_irq);
    gic::set_edge(intid);
    gic::enable(intid);

    kprintln!(
        "NET: virtio-net at {:#x}, INTID {intid}, mac {}",
        mmio.base,
        net::fmt_mac(&mac)
    );
    unsafe {
        *core::ptr::addr_of_mut!(NET) =
            Some(Net { mmio, intid, rx, tx, rx_bufs, tx_buf, mac });
    }
    mac
}

/// Transmit one raw Ethernet frame, synchronously (poll the used ring).
/// Callers hold IRQs masked, so this can't race the RX drain.
pub fn send(frame: &[u8]) {
    let Some(dev) = net() else { return };
    assert!(frame.len() <= MAX_FRAME);
    unsafe {
        core::ptr::write_bytes(dev.tx_buf as *mut u8, 0, HDR_LEN);
        core::ptr::copy_nonoverlapping(
            frame.as_ptr(),
            (dev.tx_buf + HDR_LEN) as *mut u8,
            frame.len(),
        );
    }
    dev.tx.write_desc(0, dev.tx_buf as u64, (HDR_LEN + frame.len()) as u32, 0, 0);
    dev.tx.push_avail(0);
    dev.mmio.notify(1);
    let mut spins = 0u64;
    while dev.tx.pop_used().is_none() {
        spins += 1;
        assert!(spins < 1_000_000_000, "virtio-net tx hung");
    }
}

/// IRQ path: ack, hand every received frame (minus the virtio header) to
/// the protocol stack, repost the buffer.
fn on_irq(_intid: u32) {
    let Some(dev) = net() else { return };
    dev.mmio.irq_ack();
    let mut reposted = false;
    while let Some((id, len)) = dev.rx.pop_used_len() {
        let len = len as usize;
        if len > HDR_LEN {
            let frame = unsafe {
                core::slice::from_raw_parts(
                    (dev.rx_bufs + BUF_LEN * id as usize + HDR_LEN) as *const u8,
                    (len - HDR_LEN).min(MAX_FRAME),
                )
            };
            net::on_frame(frame);
        }
        dev.rx.push_avail(id);
        reposted = true;
    }
    if reposted {
        dev.mmio.notify(0);
    }
}
