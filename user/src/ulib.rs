//! Veil user-space library: syscall wrappers, printing, program entry.
//! Syscall ABI: number in x8, args in x0..x2, result in x0 (Linux-style).

#![no_std]

use core::arch::asm;
use core::panic::PanicInfo;

// Syscall numbers — keep in sync with the kernel's syscall.rs.
pub const SYS_EXIT: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_GETPID: u64 = 2;
pub const SYS_YIELD: u64 = 3;
pub const SYS_OPEN: u64 = 4;
pub const SYS_READ: u64 = 5;
pub const SYS_CLOSE: u64 = 6;
pub const SYS_READDIR: u64 = 7;
pub const SYS_GET_ARGS: u64 = 8;

pub fn syscall(n: u64, a: u64, b: u64, c: u64) -> i64 {
    let ret: i64;
    unsafe {
        asm!(
            "svc #0",
            inlateout("x0") a => ret,
            in("x1") b,
            in("x2") c,
            in("x8") n,
            options(nostack)
        );
    }
    ret
}

pub fn exit(code: i32) -> ! {
    syscall(SYS_EXIT, code as u64, 0, 0);
    loop {} // unreachable: the kernel never schedules us again
}

pub fn write(s: &str) {
    syscall(SYS_WRITE, 1, s.as_ptr() as u64, s.len() as u64);
}

pub fn getpid() -> i64 {
    syscall(SYS_GETPID, 0, 0, 0)
}

pub fn yield_now() {
    syscall(SYS_YIELD, 0, 0, 0);
}

pub fn open(path: &str) -> i64 {
    syscall(SYS_OPEN, path.as_ptr() as u64, path.len() as u64, 0)
}

pub fn read(fd: i64, buf: &mut [u8]) -> i64 {
    syscall(SYS_READ, fd as u64, buf.as_mut_ptr() as u64, buf.len() as u64)
}

pub fn close(fd: i64) {
    syscall(SYS_CLOSE, fd as u64, 0, 0);
}

/// Directory entry `index` formatted as "NAME SIZE"; -1 when past the end.
pub fn readdir(index: u64, buf: &mut [u8]) -> i64 {
    syscall(SYS_READDIR, index, buf.as_mut_ptr() as u64, buf.len() as u64)
}

/// The argument string this program was spawned with.
pub fn get_args(buf: &mut [u8]) -> usize {
    let n = syscall(SYS_GET_ARGS, buf.as_mut_ptr() as u64, buf.len() as u64, 0);
    if n < 0 { 0 } else { n as usize }
}

pub struct Out;

impl core::fmt::Write for Out {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write(s);
        Ok(())
    }
}

#[macro_export]
macro_rules! uprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::Out, $($arg)*);
    }};
}

#[macro_export]
macro_rules! uprintln {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = writeln!($crate::Out, $($arg)*);
    }};
}

/// Defines `_start` (placed at the link base) that calls `$main` then exits.
#[macro_export]
macro_rules! entry {
    ($main:path) => {
        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".text.start")]
        pub extern "C" fn _start() -> ! {
            $main();
            $crate::exit(0)
        }
    };
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    uprintln!("user panic: {info}");
    exit(101)
}

/// Tiny ASCII number parser for argv handling.
pub fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut v: u64 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (b - b'0') as u64;
    }
    Some(v)
}
