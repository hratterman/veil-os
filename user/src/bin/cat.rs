#![no_std]
#![no_main]

use ulib::{uprint, uprintln};

ulib::entry!(main);

fn main() {
    let mut args = [0u8; 128];
    let n = ulib::get_args(&mut args);
    let Ok(path) = core::str::from_utf8(&args[..n]) else {
        return;
    };
    let path = path.trim();
    if path.is_empty() {
        uprintln!("cat: usage: cat <file>");
        return;
    }
    let fd = ulib::open(path);
    if fd < 0 {
        uprintln!("cat: {path}: not found");
        ulib::exit(1);
    }
    let mut buf = [0u8; 512];
    loop {
        let n = ulib::read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        if let Ok(s) = core::str::from_utf8(&buf[..n as usize]) {
            uprint!("{s}");
        }
    }
    ulib::close(fd);
}
