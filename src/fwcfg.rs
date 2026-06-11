//! QEMU fw_cfg over MMIO with the DMA interface — the handshake that
//! configures ramfb (M5). Base address comes from the DTB.
//!
//! Wire format gotcha (spec §5 "where it bites"): *everything* is
//! big-endian — the selector register, the DMA control block, and every
//! field of every structure read or written through it.

use crate::dtb;
use core::ptr::{read_volatile, write_volatile};

const REG_DMA: usize = 0x10; // 64-bit BE physical address of a DmaAccess

const CTL_ERROR: u32 = 1 << 0;
const CTL_READ: u32 = 1 << 1;
const CTL_SELECT: u32 = 1 << 3;
const CTL_WRITE: u32 = 1 << 4;

const KEY_SIGNATURE: u16 = 0x0000;
const KEY_FILE_DIR: u16 = 0x0019;

/// Host-visible DMA control block; all fields big-endian.
#[repr(C, align(8))]
struct DmaAccess {
    control: u32,
    length: u32,
    address: u64,
}

pub struct FwCfg {
    base: usize,
}

#[derive(Clone, Copy)]
pub struct File {
    pub select: u16,
    pub size: u32,
}

impl FwCfg {
    pub fn from_dtb(fdt: &dtb::Fdt) -> Option<FwCfg> {
        let node = fdt.find_compatible("qemu,fw-cfg-mmio")?;
        let reg = fdt.prop(node, "reg")?;
        let (addr_cells, _) = fdt.root_cells();
        let base = dtb::cells(reg, 0, addr_cells) as usize;
        let fw = FwCfg { base };
        let mut sig = [0u8; 4];
        fw.dma(
            (KEY_SIGNATURE as u32) << 16 | CTL_SELECT | CTL_READ,
            sig.as_mut_ptr(),
            4,
        )
        .ok()?;
        if &sig != b"QEMU" {
            return None;
        }
        Some(fw)
    }

    /// One DMA transaction. QEMU performs it synchronously on the register
    /// write; we still poll `control` until the device clears it.
    fn dma(&self, control: u32, buf: *mut u8, len: u32) -> Result<(), ()> {
        let mut acc = DmaAccess {
            control: control.to_be(),
            length: len.to_be(),
            address: (buf as u64).to_be(),
        };
        let acc_ptr = &mut acc as *mut DmaAccess;
        unsafe {
            core::arch::asm!("dsb sy", options(nostack)); // struct visible before kick
            write_volatile((self.base + REG_DMA) as *mut u64, (acc_ptr as u64).to_be());
            loop {
                let ctl = u32::from_be(read_volatile(core::ptr::addr_of!((*acc_ptr).control)));
                if ctl & CTL_ERROR != 0 {
                    return Err(());
                }
                if ctl == 0 {
                    // The device filled `buf` by DMA — invisible to the compiler,
                    // which in release would otherwise assume the caller's buffer
                    // is unchanged (e.g. the "QEMU" signature stayed [0;4], so
                    // from_dtb returned None → no framebuffer / no fw_cfg flags).
                    // The barrier makes the writes visible; black_box forces the
                    // optimizer to treat the pointee as clobbered so reads reload.
                    core::arch::asm!("dsb sy", options(nostack));
                    core::hint::black_box(buf);
                    return Ok(());
                }
            }
        }
    }

    fn read_selected(&self, key: u16, buf: &mut [u8]) -> Result<(), ()> {
        self.dma(
            (key as u32) << 16 | CTL_SELECT | CTL_READ,
            buf.as_mut_ptr(),
            buf.len() as u32,
        )
    }

    /// Continue reading the currently selected item (no rewind).
    fn read_more(&self, buf: &mut [u8]) -> Result<(), ()> {
        self.dma(CTL_READ, buf.as_mut_ptr(), buf.len() as u32)
    }

    /// Read (a prefix of) a file's content; returns bytes read.
    pub fn read_file(&self, file: File, buf: &mut [u8]) -> Result<usize, ()> {
        let len = (file.size as usize).min(buf.len());
        self.read_selected(file.select, &mut buf[..len])?;
        Ok(len)
    }

    /// Write a named file's content (how ramfb gets configured).
    pub fn write_file(&self, file: File, data: &[u8]) -> Result<(), ()> {
        self.dma(
            (file.select as u32) << 16 | CTL_SELECT | CTL_WRITE,
            data.as_ptr() as *mut u8,
            data.len() as u32,
        )
    }

    /// Look `name` up in the file directory (key 0x19): a BE u32 count,
    /// then per file {u32 size, u16 select, u16 rsvd, char name[56]}.
    pub fn find_file(&self, name: &str) -> Option<File> {
        let mut count_buf = [0u8; 4];
        self.read_selected(KEY_FILE_DIR, &mut count_buf).ok()?;
        let count = u32::from_be_bytes(count_buf);
        for _ in 0..count {
            let mut entry = [0u8; 64];
            self.read_more(&mut entry).ok()?;
            let len = entry[8..].iter().position(|&b| b == 0).unwrap_or(56);
            if &entry[8..8 + len] == name.as_bytes() {
                return Some(File {
                    size: u32::from_be_bytes(entry[0..4].try_into().unwrap()),
                    select: u16::from_be_bytes(entry[4..6].try_into().unwrap()),
                });
            }
        }
        None
    }
}

/// Point ramfb at `fb_pa` (XRGB8888, tightly packed rows). The 28-byte
/// config struct is, of course, all big-endian.
pub fn configure_ramfb(fw: &FwCfg, fb_pa: usize, width: u32, height: u32) -> Result<(), ()> {
    let file = fw.find_file("etc/ramfb").ok_or(())?;
    const XRGB8888: u32 = 0x3432_5258; // DRM fourcc 'XR24'
    let mut cfg = [0u8; 28];
    cfg[0..8].copy_from_slice(&(fb_pa as u64).to_be_bytes());
    cfg[8..12].copy_from_slice(&XRGB8888.to_be_bytes());
    cfg[12..16].copy_from_slice(&0u32.to_be_bytes()); // flags
    cfg[16..20].copy_from_slice(&width.to_be_bytes());
    cfg[20..24].copy_from_slice(&height.to_be_bytes());
    cfg[24..28].copy_from_slice(&(width * 4).to_be_bytes());
    fw.write_file(file, &cfg)
}
