//! AArch64 page tables (4 KiB granule, 39-bit VA, 3 levels) and MMU enable.
//!
//! Identity layout: L1[0] is a 1 GiB device block covering all virt MMIO
//! below 0x4000_0000; RAM gets normal cacheable 1 GiB blocks from
//! 0x4000_0000 up. Fine-grained 4 KiB mappings go through L2/L3 tables
//! allocated from the frame allocator (which is identity-mapped, so table
//! PAs are directly dereferenceable).
//!
//! 39-bit VA / 40-bit PA is enough through M5; the PCIe high window
//! (0x40_1000_0000) will need T0SZ=16 + an L0 level when virtio-pci
//! arrives in M6 — revisit then.

use crate::frames;

pub const PAGE_SIZE: usize = 4096;

// Descriptor bits.
const BLOCK: u64 = 0b01; // L1/L2 block entry
const TABLE: u64 = 0b11; // L1/L2 pointer to next level
const PAGE: u64 = 0b11; // L3 page entry
const ATTR_DEVICE: u64 = 0 << 2; // MAIR index 0
const ATTR_NORMAL: u64 = 1 << 2; // MAIR index 1
const AF: u64 = 1 << 10; // access flag (faults if clear!)
const SH_INNER: u64 = 0b11 << 8;
const SH_OUTER: u64 = 0b10 << 8;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;
const AP_EL0: u64 = 1 << 6; // EL0 RW (EL1 keeps RW too; no PAN on cortex-a72)

const DEVICE_FLAGS: u64 = ATTR_DEVICE | AF | SH_OUTER | PXN | UXN;
const NORMAL_FLAGS: u64 = ATTR_NORMAL | AF | SH_INNER;

// MAIR: idx 0 = Device-nGnRE, idx 1 = Normal WB write-allocate.
const MAIR: u64 = 0x04 | (0xff << 8);

// TCR: T0SZ=25 (39-bit VA), WB/WA inner+outer cacheable walks, inner
// shareable, 4K granule, TTBR1 walks disabled, 40-bit IPA.
const TCR: u64 = 25 | (1 << 8) | (1 << 10) | (3 << 12) | (1 << 23) | (2 << 32);

fn table_at(pa: usize) -> &'static mut [u64; 512] {
    unsafe { &mut *(pa as *mut [u64; 512]) }
}

pub struct Mapper {
    root: usize, // PA of the L1 table
}

impl Mapper {
    pub fn new() -> Mapper {
        Mapper {
            root: frames::alloc_zeroed().expect("no frame for L1 table"),
        }
    }

    /// Map a 1 GiB block at L1. `va` and `pa` must be 1 GiB aligned.
    pub fn map_block_1g(&mut self, va: usize, pa: usize, device: bool) {
        let flags = if device { DEVICE_FLAGS } else { NORMAL_FLAGS };
        table_at(self.root)[(va >> 30) & 0x1ff] = pa as u64 | BLOCK | flags;
    }

    /// Map a 2 MiB block at L2, creating the L1-level table on demand.
    /// `va` and `pa` must be 2 MiB aligned. Used by the Pi 4 port, where
    /// RAM and the VideoCore framebuffer share a GiB and need different
    /// memory attributes.
    pub fn map_block_2m(&mut self, va: usize, pa: usize, device: bool) {
        let flags = if device { DEVICE_FLAGS } else { NORMAL_FLAGS };
        let l1 = table_at(self.root);
        let l2_pa = Self::child_table(&mut l1[(va >> 30) & 0x1ff]);
        table_at(l2_pa)[(va >> 21) & 0x1ff] = pa as u64 | BLOCK | flags;
    }

    /// Map one 4 KiB page, creating L2/L3 tables on demand.
    pub fn map_page(&mut self, va: usize, pa: usize, device: bool) {
        let flags = if device { DEVICE_FLAGS } else { NORMAL_FLAGS };
        let l1 = table_at(self.root);
        let l2_pa = Self::child_table(&mut l1[(va >> 30) & 0x1ff]);
        let l2 = table_at(l2_pa);
        let l3_pa = Self::child_table(&mut l2[(va >> 21) & 0x1ff]);
        let l3 = table_at(l3_pa);
        l3[(va >> 12) & 0x1ff] = pa as u64 | PAGE | flags;
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "tlbi vaae1, {0}",
                "dsb ish",
                "isb",
                in(reg) (va >> 12) as u64,
                options(nostack)
            );
        }
    }

    /// Map one user-accessible 4 KiB page. Code pages are EL0-executable
    /// (UXN clear); nothing user-mapped is ever EL1-executable (PXN set).
    pub fn map_user_page(&mut self, va: usize, pa: usize, exec: bool) {
        let mut flags = ATTR_NORMAL | AF | SH_INNER | AP_EL0 | PXN;
        if !exec {
            flags |= UXN;
        }
        let l1 = table_at(self.root);
        let l2_pa = Self::child_table(&mut l1[(va >> 30) & 0x1ff]);
        let l3_pa = Self::child_table(&mut table_at(l2_pa)[(va >> 21) & 0x1ff]);
        table_at(l3_pa)[(va >> 12) & 0x1ff] = pa as u64 | PAGE | flags;
        unsafe {
            core::arch::asm!("dsb ishst", "isb", options(nostack));
        }
    }

    /// New address space for a process: fresh root with the kernel's
    /// identity entries (device + RAM 1 GiB blocks) copied in, so kernel
    /// code/data/MMIO stay mapped (EL0 can't touch them: no AP_EL0 bit).
    pub fn clone_kernel(kernel_root: usize) -> Mapper {
        let mapper = Mapper::new();
        let src = table_at(kernel_root);
        let dst = table_at(mapper.root);
        for i in 0..4 {
            dst[i] = src[i];
        }
        mapper
    }

    /// Free every user mapping (L1 index >= 4: identity blocks live below)
    /// including the intermediate tables and the root itself. Only call on
    /// process teardown, with some other root loaded in TTBR0.
    pub fn free_user_space(self) {
        let table_pa = |entry: u64| (entry & 0x0000_ffff_ffff_f000) as usize;
        let l1 = table_at(self.root);
        for l1e in l1.iter().skip(4) {
            if *l1e & 0b11 != TABLE {
                continue;
            }
            let l2 = table_at(table_pa(*l1e));
            for l2e in l2.iter() {
                if *l2e & 0b11 != TABLE {
                    continue;
                }
                let l3 = table_at(table_pa(*l2e));
                for l3e in l3.iter() {
                    if *l3e & 1 != 0 {
                        frames::free(table_pa(*l3e), 1);
                    }
                }
                frames::free(table_pa(*l2e), 1);
            }
            frames::free(table_pa(*l1e), 1);
        }
        frames::free(self.root, 1);
    }

    pub fn root(&self) -> usize {
        self.root
    }

    fn child_table(entry: &mut u64) -> usize {
        if *entry & 1 == 0 {
            let pa = frames::alloc_zeroed().expect("no frame for page table");
            *entry = pa as u64 | TABLE;
            pa
        } else {
            assert!(*entry & 0b11 == TABLE, "remapping over a block entry");
            (*entry & 0x0000_ffff_ffff_f000) as usize
        }
    }

    /// Identity-map the machine: device space below RAM, all of RAM normal.
    pub fn identity_map_machine(&mut self, ram_base: usize, ram_size: usize) {
        self.map_block_1g(0, 0, true);
        let gib = 1 << 30;
        let mut offset = 0;
        while offset < ram_size {
            self.map_block_1g(ram_base + offset, ram_base + offset, false);
            offset += gib;
        }
    }

    /// Switch address spaces (no ASIDs yet: full TLB flush each time).
    /// Enable the MMU + caches on a secondary core, using the kernel's existing
    /// page tables (`root`). Mirrors `enable()` but takes the root directly.
    pub fn enable_at(root: usize) {
        unsafe {
            core::arch::asm!(
                "msr mair_el1, {mair}",
                "msr tcr_el1, {tcr}",
                "msr ttbr0_el1, {ttbr}",
                "dsb ish",
                "isb",
                "tlbi vmalle1",
                "dsb ish",
                "isb",
                "mrs {sctlr}, sctlr_el1",
                "orr {sctlr}, {sctlr}, #1",
                "orr {sctlr}, {sctlr}, #(1 << 2)",
                "orr {sctlr}, {sctlr}, #(1 << 12)",
                "msr sctlr_el1, {sctlr}",
                "isb",
                mair = in(reg) MAIR,
                tcr = in(reg) TCR,
                ttbr = in(reg) root as u64,
                sctlr = out(reg) _,
                options(nostack)
            );
        }
    }

    pub fn switch_ttbr0(root: usize) {
        unsafe {
            core::arch::asm!(
                "msr ttbr0_el1, {0}",
                "dsb ish",
                "tlbi vmalle1",
                "dsb ish",
                "isb",
                in(reg) root as u64,
                options(nostack)
            );
        }
    }

    /// Point TTBR0 at our tables and switch on MMU + caches. IRQs are
    /// masked across the sequence so no exception runs half-configured.
    pub fn enable(&self) {
        unsafe {
            core::arch::asm!(
                "mrs {tmp}, daif",
                "msr daifset, #2",
                "msr mair_el1, {mair}",
                "msr tcr_el1, {tcr}",
                "msr ttbr0_el1, {ttbr}",
                "dsb ish",
                "isb",
                "tlbi vmalle1",
                "dsb ish",
                "isb",
                "mrs {sctlr}, sctlr_el1",
                "orr {sctlr}, {sctlr}, #1",        // M: MMU on
                "orr {sctlr}, {sctlr}, #(1 << 2)", // C: data cache
                "orr {sctlr}, {sctlr}, #(1 << 12)",// I: instruction cache
                "msr sctlr_el1, {sctlr}",
                "isb",
                "msr daif, {tmp}",
                mair = in(reg) MAIR,
                tcr = in(reg) TCR,
                ttbr = in(reg) self.root as u64,
                sctlr = out(reg) _,
                tmp = out(reg) _,
                options(nostack)
            );
        }
    }
}
