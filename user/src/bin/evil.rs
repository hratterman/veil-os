//! Negative test for the M9 privilege boundary: reading kernel memory
//! from EL0 must be fatal (the kernel kills us with a fault), never
//! silently succeed.

#![no_std]
#![no_main]

use ulib::uprintln;

ulib::entry!(main);

fn main() {
    uprintln!("evil: about to read kernel memory from EL0...");
    let kernel_word = unsafe { core::ptr::read_volatile(0x4010_0000 as *const u64) };
    // If we get here, the privilege boundary does not exist.
    uprintln!("evil: PRIVILEGE BREACH read {kernel_word:#x}");
}
