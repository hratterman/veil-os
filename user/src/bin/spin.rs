//! Preemption proof: busy-loops WITHOUT yielding. If the shell stays
//! responsive while this runs, multitasking is genuinely preemptive.

#![no_std]
#![no_main]

use ulib::uprintln;

ulib::entry!(main);

fn main() {
    let mut args = [0u8; 32];
    let n = ulib::get_args(&mut args);
    let beats = core::str::from_utf8(&args[..n])
        .ok()
        .and_then(ulib::parse_u64)
        .unwrap_or(8);
    let pid = ulib::getpid();
    for i in 1..=beats {
        let mut acc: u64 = 0;
        for j in 0..200_000_000u64 {
            acc = acc.wrapping_add(core::hint::black_box(j));
        }
        uprintln!("spin[{pid}]: beat {i}/{beats} (acc={acc:#x})");
    }
    uprintln!("spin[{pid}]: done");
}
