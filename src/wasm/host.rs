//! WASM host imports — enough WASI to run a hello-world: `fd_write` (collect
//! the bytes a guest writes to stdout/stderr), `proc_exit`, and a couple of
//! debug shims. Unknown imports are no-ops that return 0.

use alloc::string::String;
use alloc::vec::Vec;

pub struct Host {
    pub output: String,
}

impl Host {
    pub fn new() -> Host {
        Host { output: String::new() }
    }

    /// Dispatch an imported function call. Returns its (single) i32/i64 result.
    pub fn call(&mut self, field: &str, args: &[i64], mem: &mut [u8]) -> Option<i64> {
        match field {
            // wasi_snapshot_preview1.fd_write(fd, iovs, iovs_len, nwritten) -> errno
            "fd_write" => {
                let iovs = *args.get(1)? as u32 as usize;
                let iovs_len = *args.get(2)? as u32 as usize;
                let nwritten = *args.get(3)? as u32 as usize;
                let mut total = 0usize;
                for i in 0..iovs_len {
                    let base = iovs + i * 8;
                    let ptr = u32::from_le_bytes(mem.get(base..base + 4)?.try_into().ok()?) as usize;
                    let len = u32::from_le_bytes(mem.get(base + 4..base + 8)?.try_into().ok()?) as usize;
                    if let Some(s) = mem.get(ptr..ptr + len) {
                        self.output.push_str(&String::from_utf8_lossy(s));
                        total += len;
                    }
                }
                if let Some(slot) = mem.get_mut(nwritten..nwritten + 4) {
                    slot.copy_from_slice(&(total as u32).to_le_bytes());
                }
                Some(0)
            }
            "proc_exit" => Some(0),
            // Simple debug shims some toolchains/hand-written modules import.
            "print_i32" | "print" => {
                if let Some(v) = args.first() {
                    let mut s = String::new();
                    let _ = core::fmt::write(&mut s, format_args!("{}\n", *v as i32));
                    self.output.push_str(&s);
                }
                Some(0)
            }
            "print_char" => {
                if let Some(&v) = args.first() {
                    self.output.push((v as u8) as char);
                }
                Some(0)
            }
            _ => Some(0),
        }
    }
}

/// Convenience: read a NUL-terminated string from guest memory.
pub fn read_cstr(mem: &[u8], ptr: usize) -> String {
    let end = mem[ptr..].iter().position(|&b| b == 0).map(|i| ptr + i).unwrap_or(mem.len());
    String::from_utf8_lossy(&mem[ptr..end]).into_owned()
}

#[allow(dead_code)]
fn _unused(_: Vec<u8>) {}
