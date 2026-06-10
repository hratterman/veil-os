//! Kernel heap: an address-ordered free-list allocator with first-fit,
//! split and coalesce, registered as the global allocator so `Box`, `Vec`,
//! `String`, `BTreeMap` work. Single core; IRQs are masked inside the
//! allocator so interrupt handlers may allocate safely.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use core::ptr;

const MIN_BLOCK: usize = 16; // sizeof(Node), also min alignment

#[repr(C)]
struct Node {
    size: usize,
    next: *mut Node,
}

struct FreeList {
    head: *mut Node,
    free: usize,
}

pub struct LockedHeap(UnsafeCell<FreeList>);

// Single-core with IRQs masked inside `critical` — no data races possible.
unsafe impl Sync for LockedHeap {}

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap(UnsafeCell::new(FreeList {
    head: ptr::null_mut(),
    free: 0,
}));

fn critical<R>(f: impl FnOnce() -> R) -> R {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack));
        core::arch::asm!("msr daifset, #2", options(nomem, nostack));
    }
    let result = f();
    unsafe { core::arch::asm!("msr daif, {}", in(reg) daif, options(nomem, nostack)) };
    result
}

/// Both alloc and dealloc must normalize a Layout identically, so a block
/// carved at alloc time is returned at exactly the same size.
fn normalize(layout: Layout) -> (usize, usize) {
    let size = layout.size().max(1).next_multiple_of(MIN_BLOCK);
    let align = layout.align().max(MIN_BLOCK);
    (size, align)
}

impl FreeList {
    /// Insert [start, start+size) keeping the list address-sorted and
    /// coalescing with both neighbors.
    unsafe fn insert(&mut self, start: usize, size: usize) {
        self.free += size;
        let mut prev: *mut *mut Node = &mut self.head;
        unsafe {
            while !(*prev).is_null() && ((*prev) as usize) < start {
                prev = &mut (**prev).next;
            }
            let next = *prev;
            let node = start as *mut Node;
            (*node).size = size;
            (*node).next = next;
            *prev = node;
            // Coalesce forward.
            if !next.is_null() && start + size == next as usize {
                (*node).size += (*next).size;
                (*node).next = (*next).next;
            }
            // Coalesce backward (prev points either at head or into a node).
            if prev != &mut self.head {
                let before = (prev as usize - core::mem::offset_of!(Node, next)) as *mut Node;
                if before as usize + (*before).size == start {
                    (*before).size += (*node).size;
                    (*before).next = (*node).next;
                }
            }
        }
    }
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let (size, align) = normalize(layout);
        critical(|| unsafe {
            let fl = &mut *self.0.get();
            let mut prev: *mut *mut Node = &mut fl.head;
            while !(*prev).is_null() {
                let block = *prev;
                let bstart = block as usize;
                let bsize = (*block).size;
                let aligned = bstart.next_multiple_of(align);
                if aligned + size <= bstart + bsize {
                    // Take the block out, give back the unused ends.
                    *prev = (*block).next;
                    fl.free -= bsize;
                    let front = aligned - bstart;
                    let back = (bstart + bsize) - (aligned + size);
                    if front > 0 {
                        fl.insert(bstart, front);
                    }
                    if back > 0 {
                        fl.insert(aligned + size, back);
                    }
                    return aligned as *mut u8;
                }
                prev = &mut (*block).next;
            }
            ptr::null_mut() // OOM: default error handler panics
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let (size, _) = normalize(layout);
        critical(|| unsafe {
            let fl = &mut *self.0.get();
            fl.insert(ptr as usize, size);
        })
    }
}

/// Donate [start, start+size) — identity-mapped normal RAM — to the heap.
pub fn init(start: usize, size: usize) {
    critical(|| unsafe {
        let fl = &mut *ALLOCATOR.0.get();
        fl.insert(start, size);
    })
}

pub fn free_bytes() -> usize {
    critical(|| unsafe { (*ALLOCATOR.0.get()).free })
}
