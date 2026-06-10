#![no_std]
#![no_main]

use ulib::uprintln;

ulib::entry!(main);

fn main() {
    let mut buf = [0u8; 64];
    let mut i = 0;
    loop {
        let n = ulib::readdir(i, &mut buf);
        if n < 0 {
            break;
        }
        if let Ok(line) = core::str::from_utf8(&buf[..n as usize]) {
            uprintln!("{line}");
        }
        i += 1;
    }
}
