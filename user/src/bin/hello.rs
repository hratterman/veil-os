//! M9 proof program: runs at EL0, does real work, talks to the kernel
//! only through SVC.

#![no_std]
#![no_main]

use ulib::uprintln;

ulib::entry!(main);

fn main() {
    let pid = ulib::getpid();
    // A computation the compiler can't fold away across the syscall.
    let mut sum: u64 = 0;
    for i in 1..=1000u64 {
        sum = sum.wrapping_add(core::hint::black_box(i * i));
    }
    uprintln!("hello from EL0! pid={pid} sum(1..=1000 sq)={sum}");
    uprintln!("hello: making a second syscall, then exiting cleanly");
}
