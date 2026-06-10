#![no_std]
#![no_main]

use ulib::uprintln;

ulib::entry!(main);

fn main() {
    let mut args = [0u8; 256];
    let n = ulib::get_args(&mut args);
    let s = core::str::from_utf8(&args[..n]).unwrap_or("");
    uprintln!("{s}");
}
