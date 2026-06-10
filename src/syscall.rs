//! Syscall dispatch (SVC from EL0) and the kernel console buffer that
//! carries user write() output to the shell window.
//!
//! ABI: number in x8, args x0..x2, result in x0. Numbers match user/ulib.

use crate::exceptions::TrapFrame;
use crate::scheduler::{self, File, USER_BASE};
use crate::{fs, kprint, kprintln};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;

const USER_LIMIT: usize = 0x20_0100_0000; // == scheduler USTACK_TOP

const ENOSYS: i64 = -38;
const EBADF: i64 = -9;
const EFAULT: i64 = -14;
const ENOENT: i64 = -2;

/// Validate a user buffer range and hand back a kernel-visible slice
/// (current TTBR0 maps user pages; EL1 may access them — no PAN on A72).
fn user_slice<'a>(ptr: u64, len: u64) -> Option<&'a [u8]> {
    let (ptr, len) = (ptr as usize, len as usize);
    if ptr < USER_BASE || ptr.checked_add(len)? > USER_LIMIT {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts(ptr as *const u8, len) })
}

fn user_slice_mut<'a>(ptr: u64, len: u64) -> Option<&'a mut [u8]> {
    let (ptr, len) = (ptr as usize, len as usize);
    if ptr < USER_BASE || ptr.checked_add(len)? > USER_LIMIT {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts_mut(ptr as *mut u8, len) })
}

pub fn dispatch(tf: &mut TrapFrame) {
    let (a, b, c) = (tf.x[0], tf.x[1], tf.x[2]);
    let ret: i64 = match tf.x[8] {
        0 => scheduler::exit_current(a as i32),
        1 => sys_write(a, b, c),
        2 => scheduler::with_current(|t| t.pid as i64),
        3 => {
            scheduler::yield_now();
            0
        }
        4 => sys_open(a, b),
        5 => sys_read(a, b, c),
        6 => sys_close(a),
        7 => sys_readdir(a, b, c),
        8 => sys_get_args(b_ptr(a), b),
        _ => ENOSYS,
    };
    tf.x[0] = ret as u64;
}

// get_args passes (buf, cap) in x0/x1 — tiny shim for arg naming clarity.
fn b_ptr(x: u64) -> u64 {
    x
}

fn sys_write(fd: u64, ptr: u64, len: u64) -> i64 {
    if fd != 1 {
        return EBADF;
    }
    let Some(buf) = user_slice(ptr, len.min(4096)) else {
        return EFAULT;
    };
    let Ok(s) = core::str::from_utf8(buf) else {
        return EFAULT;
    };
    kprint!("{s}"); // serial mirror (test assertions read this)
    console_write(s);
    buf.len() as i64
}

fn sys_open(ptr: u64, len: u64) -> i64 {
    let Some(buf) = user_slice(ptr, len.min(64)) else {
        return EFAULT;
    };
    let Ok(name) = core::str::from_utf8(buf) else {
        return EFAULT;
    };
    let Some(data) = fs::read_file(name) else {
        return ENOENT;
    };
    scheduler::with_current(|t| {
        let file = Some(File { data, pos: 0 });
        for (i, slot) in t.fds.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = file;
                return i as i64;
            }
        }
        t.fds.push(file);
        (t.fds.len() - 1) as i64
    })
}

fn sys_read(fd: u64, ptr: u64, len: u64) -> i64 {
    let Some(buf) = user_slice_mut(ptr, len) else {
        return EFAULT;
    };
    scheduler::with_current(|t| {
        let Some(Some(file)) = t.fds.get_mut(fd as usize) else {
            return EBADF;
        };
        let n = buf.len().min(file.data.len() - file.pos);
        buf[..n].copy_from_slice(&file.data[file.pos..file.pos + n]);
        file.pos += n;
        n as i64
    })
}

fn sys_close(fd: u64) -> i64 {
    scheduler::with_current(|t| match t.fds.get_mut(fd as usize) {
        Some(slot @ Some(_)) => {
            *slot = None;
            0
        }
        _ => EBADF,
    })
}

fn sys_readdir(index: u64, ptr: u64, cap: u64) -> i64 {
    let Some(buf) = user_slice_mut(ptr, cap) else {
        return EFAULT;
    };
    let Some(entries) = fs::list_root() else {
        return ENOENT;
    };
    let Some((name, size)) = entries.get(index as usize) else {
        return -1; // past the end
    };
    let mut line = String::new();
    let _ = write!(line, "{name} {size}");
    let n = line.len().min(buf.len());
    buf[..n].copy_from_slice(&line.as_bytes()[..n]);
    n as i64
}

fn sys_get_args(ptr: u64, cap: u64) -> i64 {
    let Some(buf) = user_slice_mut(ptr, cap) else {
        return EFAULT;
    };
    scheduler::with_current(|t| {
        let n = t.args.len().min(buf.len());
        buf[..n].copy_from_slice(&t.args.as_bytes()[..n]);
        n as i64
    })
}

// --- console buffer ----------------------------------------------------

static mut CONSOLE: Option<String> = None;

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

fn console_write(s: &str) {
    critical(|| unsafe {
        let console = &mut *core::ptr::addr_of_mut!(CONSOLE);
        let buf = console.get_or_insert_with(String::new);
        if buf.len() < 16384 {
            buf.push_str(s);
        }
    })
}

/// Drain everything user programs wrote since the last call (shell pump).
pub fn console_take() -> Option<String> {
    critical(|| unsafe { (*core::ptr::addr_of_mut!(CONSOLE)).take() })
}
