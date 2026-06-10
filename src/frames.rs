//! Physical frame allocator: one bit per 4 KiB frame, next-fit scan.
//! Sized for up to 4 GiB of RAM (128 KiB of .bss); actual RAM size comes
//! from the DTB at init.

use core::sync::atomic::{AtomicUsize, Ordering};

pub const FRAME_SIZE: usize = 4096;
const MAX_FRAMES: usize = (4 << 30) / FRAME_SIZE;

// 1 = used. Statics are mutated only inside `with` (single core, IRQs off).
static mut BITMAP: [u64; MAX_FRAMES / 64] = [0; MAX_FRAMES / 64];
static mut RAM_BASE: usize = 0;
static mut NFRAMES: usize = 0;
static mut CURSOR: usize = 0;
static FREE: AtomicUsize = AtomicUsize::new(0);

/// Run `f` with exclusive access to the allocator state (masks IRQs).
fn with<R>(f: impl FnOnce() -> R) -> R {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack));
        core::arch::asm!("msr daifset, #2", options(nomem, nostack));
    }
    let result = f();
    unsafe { core::arch::asm!("msr daif, {}", in(reg) daif, options(nomem, nostack)) };
    result
}

fn mark(frame: usize, used: bool) {
    unsafe {
        let word = &mut (*core::ptr::addr_of_mut!(BITMAP))[frame / 64];
        let bit = 1u64 << (frame % 64);
        if used {
            debug_assert!(*word & bit == 0, "double-allocating frame {frame}");
            *word |= bit;
        } else {
            debug_assert!(*word & bit != 0, "double-freeing frame {frame}");
            *word &= !bit;
        }
    }
}

fn is_used(frame: usize) -> bool {
    unsafe { (*core::ptr::addr_of!(BITMAP))[frame / 64] & (1 << (frame % 64)) != 0 }
}

/// `reserved` ranges (physical, half-open) are marked used forever —
/// the DTB blob and the kernel image + stack.
pub fn init(ram_base: usize, ram_size: usize, reserved: &[(usize, usize)]) {
    with(|| {
        let nframes = (ram_size / FRAME_SIZE).min(MAX_FRAMES);
        unsafe {
            RAM_BASE = ram_base;
            NFRAMES = nframes;
        }
        FREE.store(nframes, Ordering::Relaxed);
        for &(start, end) in reserved {
            if start >= end {
                continue; // empty range (e.g. "no DTB" on the Pi)
            }
            let first = start.saturating_sub(ram_base) / FRAME_SIZE;
            let last = (end - 1).saturating_sub(ram_base) / FRAME_SIZE;
            for frame in first..=last.min(nframes - 1) {
                if !is_used(frame) {
                    // ranges may overlap (kernel + DTB on the Pi)
                    mark(frame, true);
                    FREE.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }
    })
}

pub fn free_frames() -> usize {
    FREE.load(Ordering::Relaxed)
}

/// Allocate `count` physically contiguous frames; returns the base PA.
pub fn alloc_contiguous(count: usize) -> Option<usize> {
    with(|| {
        let (nframes, base, start_at) = unsafe { (NFRAMES, RAM_BASE, CURSOR) };
        // Next-fit with wraparound: two passes over the bitmap at most.
        let mut run = 0;
        let mut scanned = 0;
        let mut frame = start_at;
        while scanned < 2 * nframes {
            if frame >= nframes {
                frame = 0;
                run = 0; // runs don't wrap
            }
            run = if is_used(frame) { 0 } else { run + 1 };
            frame += 1;
            scanned += 1;
            if run == count {
                let first = frame - count;
                for f in first..frame {
                    mark(f, true);
                }
                FREE.fetch_sub(count, Ordering::Relaxed);
                unsafe { CURSOR = frame };
                return Some(base + first * FRAME_SIZE);
            }
        }
        None
    })
}

pub fn alloc() -> Option<usize> {
    alloc_contiguous(1)
}

/// Allocate one frame and zero it (page tables need this).
pub fn alloc_zeroed() -> Option<usize> {
    let pa = alloc()?;
    unsafe { core::ptr::write_bytes(pa as *mut u8, 0, FRAME_SIZE) };
    Some(pa)
}

pub fn free(pa: usize, count: usize) {
    with(|| {
        let base = unsafe { RAM_BASE };
        let first = (pa - base) / FRAME_SIZE;
        for f in first..first + count {
            mark(f, false);
        }
        FREE.fetch_add(count, Ordering::Relaxed);
    })
}
