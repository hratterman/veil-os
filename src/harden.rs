//! M41 step 18: kernel hardening.
//!
//! * **Stack canary** — a magic word at the bottom of the boot stack; a
//!   stack overflow would overwrite it, and a check catches that.
//! * **ASLR** — the kernel heap is based at a per-boot-random page offset.
//! * **W^X** — a controlled test: map a page writable-but-non-executable
//!   (PXN/UXN), write code into it, then try to execute it. The instruction
//!   fetch faults; the exception handler catches it (via `EXPECT_FAULT`) and
//!   returns cleanly. Proves writable memory cannot be executed.
//! * **Null-pointer guard** — user address spaces never map VA 0, so an EL0
//!   null dereference faults (proven in step 15's process-kill path).

use core::sync::atomic::{AtomicBool, Ordering};

unsafe extern "C" {
    static __stack_bottom: u8;
}

const CANARY: u64 = 0x5645_494c_4341_4e41; // "VEILCANA"

/// Controlled-fault recovery: when a synchronous fault is *expected* (the W^X
/// test below), the handler sets FAULT_HIT and returns via LR instead of
/// panicking. Single-threaded boot, so plain atomics are enough.
pub static EXPECT_FAULT: AtomicBool = AtomicBool::new(false);
pub static FAULT_HIT: AtomicBool = AtomicBool::new(false);

/// Write the canary at the bottom (overflow edge) of the boot stack.
pub fn install_stack_canary() {
    unsafe {
        let p = core::ptr::addr_of!(__stack_bottom) as *mut u64;
        p.write_volatile(CANARY);
    }
}

fn stack_canary_ok() -> bool {
    unsafe {
        let p = core::ptr::addr_of!(__stack_bottom) as *const u64;
        p.read_volatile() == CANARY
    }
}

/// A per-boot random value from the cycle counter (entropy source).
pub fn rand_u64() -> u64 {
    let v: u64;
    unsafe { core::arch::asm!("mrs {}, cntvct_el0", out(reg) v, options(nomem, nostack)) };
    // mix the low bits a little so successive reads differ more
    v.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (v >> 7)
}

/// W^X test: a writable, non-executable page must fault when executed.
fn wx_test() -> bool {
    use crate::paging::Mapper;
    let kroot = crate::scheduler::kernel_root();
    let Some(pa) = crate::frames::alloc_contiguous(1) else { return false };
    let mut m = Mapper::clone_kernel(kroot);
    const TEST_VA: usize = crate::scheduler::USER_BASE; // a user-range VA, unused
    m.map_user_page(TEST_VA, pa, false); // AP_EL0 + PXN + UXN: writable, non-exec
    let root = m.root();
    Mapper::switch_ttbr0(root);

    // Write a single `ret` (0xd65f03c0) into the page — it's writable.
    unsafe {
        (TEST_VA as *mut u32).write_volatile(0xd65f_03c0);
        core::arch::asm!("dsb ish", "isb", options(nostack));
    }

    EXPECT_FAULT.store(true, Ordering::SeqCst);
    FAULT_HIT.store(false, Ordering::SeqCst);
    // Try to execute the writable page: the instruction fetch hits PXN and
    // faults; the handler returns us here (via LR).
    unsafe {
        let f: extern "C" fn() = core::mem::transmute(TEST_VA);
        f();
    }
    EXPECT_FAULT.store(false, Ordering::SeqCst);
    let hit = FAULT_HIT.load(Ordering::SeqCst);

    Mapper::switch_ttbr0(kroot); // restore the kernel address space
    hit
}

/// Run the hardening self-tests and emit the proof tokens.
pub fn selftest() {
    // Stack canary.
    if stack_canary_ok() {
        crate::kprintln!("STACK_CANARY_OK: boot-stack guard intact (overflow would trip it)");
    } else {
        crate::kprintln!("STACK_CANARY_FAIL: the stack canary was corrupted!");
    }

    // W^X.
    let wx = wx_test();
    if wx {
        crate::kprintln!("WXN_OK: executing a writable (non-exec) page faulted cleanly — W^X enforced");
    } else {
        crate::kprintln!("WXN_FAIL: writable memory was executable!");
    }
}
