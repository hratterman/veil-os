//! WASM host imports — enough WASI to run a hello-world: `fd_write` (collect
//! the bytes a guest writes to stdout/stderr), `proc_exit`, and a couple of
//! debug shims. Unknown imports are no-ops that return 0.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// A host-owned drawing surface a graphical WASM app renders into via the
/// `veil_*` graphics ABI (M41 step 12). The app window blits it after each call.
pub struct HostFb {
    pub px: Vec<u32>,
    pub w: usize,
    pub h: usize,
}

pub struct Host {
    pub output: String,
    /// Open TCP sockets opened by `veil_tcp_connect`, indexed by the handle the
    /// guest holds. (M41 step 11: network access for WASM apps.)
    sockets: Vec<Option<crate::net::Handle>>,
    /// Drawing surface for a graphical app (None for plain WASI modules).
    pub fb: Option<HostFb>,
    /// M41 step 16: granted capabilities (perms::*) + the app name (for logs).
    /// Protected host calls are refused here when the bit isn't set.
    pub perms: u32,
    pub app_name: String,
}

impl Host {
    /// Refuse a protected host call the app lacks permission for. Returns the
    /// rejection value if denied, or None if allowed.
    fn deny(&self, cap: u32) -> Option<i64> {
        if self.perms & cap == 0 {
            crate::kprintln!(
                "PERM_DENIED: {} blocked a {} call from {}",
                "kernel",
                crate::perms::name(cap),
                self.app_name
            );
            Some(-1)
        } else {
            None
        }
    }
}

/// Read a guest string given (ptr, len) i64 args.
fn mem_str(mem: &[u8], ptr: i64, len: i64) -> Option<String> {
    let (p, l) = (ptr as u32 as usize, len as u32 as usize);
    mem.get(p..p + l).map(|s| String::from_utf8_lossy(s).into_owned())
}

/// Parse a dotted-quad IPv4, else None.
fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.split('.');
    for o in &mut out {
        *o = parts.next()?.parse().ok()?;
    }
    if parts.next().is_some() { None } else { Some(out) }
}

impl Host {
    pub fn new() -> Host {
        Host { output: String::new(), sockets: Vec::new(), fb: None, perms: crate::perms::ALL, app_name: String::from("system") }
    }

    /// A host with a `w*h` drawing surface (for graphical apps).
    pub fn new_graphical(w: usize, h: usize) -> Host {
        Host {
            output: String::new(),
            sockets: Vec::new(),
            fb: Some(HostFb { px: vec![0xff14_1414; w * h], w, h }),
            perms: crate::perms::ALL,
            app_name: String::from("system"),
        }
    }

    /// Dispatch an imported function call. Returns its (single) i32/i64 result.
    pub fn call(&mut self, field: &str, args: &[i64], mem: &mut [u8]) -> Option<i64> {
        match field {
            // ---- M41 step 12: Veil graphics / storage / log ABI ----
            "veil_width" => Some(self.fb.as_ref().map(|f| f.w as i64).unwrap_or(0)),
            "veil_height" => Some(self.fb.as_ref().map(|f| f.h as i64).unwrap_or(0)),
            "veil_clear" => {
                if let Some(f) = self.fb.as_mut() {
                    let c = *args.first()? as u32;
                    for p in f.px.iter_mut() {
                        *p = c | 0xff00_0000;
                    }
                }
                Some(0)
            }
            "veil_fill_rect" => {
                if let Some(f) = self.fb.as_mut() {
                    let fb = unsafe { crate::fb::Framebuffer::new(f.px.as_mut_ptr(), f.w, f.h, f.w * 4) };
                    let (x, y, w, h) = (*args.first()? as isize, *args.get(1)? as isize, *args.get(2)? as isize, *args.get(3)? as isize);
                    let color = *args.get(4)? as u32 | 0xff00_0000;
                    if x >= 0 && y >= 0 && w > 0 && h > 0 {
                        fb.fill_rect(x as usize, y as usize, (w as usize).min(f.w), (h as usize).min(f.h), color);
                    }
                }
                Some(0)
            }
            "veil_draw_text" => {
                let s = mem_str(mem, *args.get(2)?, *args.get(3)?)?;
                if let Some(f) = self.fb.as_mut() {
                    let fb = unsafe { crate::fb::Framebuffer::new(f.px.as_mut_ptr(), f.w, f.h, f.w * 4) };
                    let (x, y) = (*args.first()? as isize, *args.get(1)? as isize);
                    let color = *args.get(4)? as u32 | 0xff00_0000;
                    let size = (*args.get(5).unwrap_or(&16) as u16).clamp(8, 64);
                    if x >= 0 && y >= 0 {
                        fb.draw_text(x as usize, y as usize, &s, crate::freetype::FontId::Ui, size, color);
                    }
                }
                Some(0)
            }
            "veil_log" => {
                let s = mem_str(mem, *args.first()?, *args.get(1)?)?;
                crate::kprintln!("WASM_APP: {s}");
                self.output.push_str(&s);
                self.output.push('\n');
                Some(0)
            }
            // M41 step 21: the C compiler's print_int — format an int and emit it.
            "veil_print_int" => {
                let n = *args.first()? as i32;
                let s = alloc::format!("{n}");
                crate::kprintln!("WASM_APP: {s}");
                self.output.push_str(&s);
                self.output.push('\n');
                Some(0)
            }
            "veil_store_set" => {
                if let Some(r) = self.deny(crate::perms::FILESYSTEM) {
                    return Some(r);
                }
                let k = mem_str(mem, *args.first()?, *args.get(1)?)?;
                let v = mem_str(mem, *args.get(2)?, *args.get(3)?)?;
                crate::browser::storage_set(true, "wasmapp", &k, &v);
                Some(0)
            }
            "veil_store_get" => {
                if let Some(r) = self.deny(crate::perms::FILESYSTEM) {
                    return Some(r);
                }
                let k = mem_str(mem, *args.first()?, *args.get(1)?)?;
                let out = *args.get(2)? as u32 as usize;
                let cap = *args.get(3)? as u32 as usize;
                let v = crate::browser::storage_get(true, "wasmapp", &k).unwrap_or_default();
                let b = v.as_bytes();
                let n = b.len().min(cap);
                if let Some(slot) = mem.get_mut(out..out + n) {
                    slot.copy_from_slice(&b[..n]);
                }
                Some(b.len() as i64)
            }
            "veil_beep" => {
                if let Some(r) = self.deny(crate::perms::AUDIO) {
                    return Some(r);
                }
                Some(0) // audio tone — reserved in the ABI, no-op here
            }
            // ---- M41 step 11: Veil network host functions ----
            // veil_http_get(url_ptr, url_len, out_ptr, out_cap) -> body length
            // (full length even if truncated to out_cap; -1 on failure).
            "veil_http_get" | "veil_http_post" => {
                if let Some(r) = self.deny(crate::perms::NETWORK) {
                    return Some(r);
                }
                let url = mem_str(mem, *args.first()?, *args.get(1)?)?;
                let (out_ptr, out_cap, body);
                if field == "veil_http_post" {
                    let b = mem_str(mem, *args.get(2)?, *args.get(3)?)?;
                    out_ptr = *args.get(4)? as u32 as usize;
                    out_cap = *args.get(5)? as u32 as usize;
                    body = crate::browser::shell_fetch(&url, Some(b.as_bytes()));
                } else {
                    out_ptr = *args.get(2)? as u32 as usize;
                    out_cap = *args.get(3)? as u32 as usize;
                    body = crate::browser::shell_fetch(&url, None);
                }
                match body {
                    Some((status, data)) => {
                        crate::kprintln!("WASM_NET: {field} {url} -> {status} ({} bytes)", data.len());
                        let n = data.len().min(out_cap);
                        if let Some(slot) = mem.get_mut(out_ptr..out_ptr + n) {
                            slot.copy_from_slice(&data[..n]);
                        }
                        Some(data.len() as i64)
                    }
                    None => {
                        crate::kprintln!("WASM_NET: {field} {url} failed (no network?)");
                        Some(-1)
                    }
                }
            }
            // veil_dns_resolve(host_ptr, host_len) -> packed big-endian IPv4 (-1)
            "veil_dns_resolve" => {
                if let Some(r) = self.deny(crate::perms::NETWORK) {
                    return Some(r);
                }
                let host = mem_str(mem, *args.first()?, *args.get(1)?)?;
                match parse_ipv4(&host).or_else(|| crate::net::dns_resolve(&host)) {
                    Some(ip) => Some(u32::from_be_bytes(ip) as i64),
                    None => Some(-1),
                }
            }
            // veil_tcp_connect(host_ptr, host_len, port) -> socket handle (-1)
            "veil_tcp_connect" => {
                if let Some(r) = self.deny(crate::perms::NETWORK) {
                    return Some(r);
                }
                let host = mem_str(mem, *args.first()?, *args.get(1)?)?;
                let port = *args.get(2)? as u16;
                let ip = parse_ipv4(&host).or_else(|| crate::net::dns_resolve(&host))?;
                match crate::net::tcp_connect(ip, port) {
                    Some(h) => {
                        self.sockets.push(Some(h));
                        crate::kprintln!("WASM_NET: tcp_connect {host}:{port} -> sock {}", self.sockets.len() - 1);
                        Some((self.sockets.len() - 1) as i64)
                    }
                    None => Some(-1),
                }
            }
            // veil_tcp_send(sock, ptr, len) -> bytes written (-1)
            "veil_tcp_send" => {
                let sock = *args.first()? as usize;
                let ptr = *args.get(1)? as u32 as usize;
                let len = *args.get(2)? as u32 as usize;
                let h = (*self.sockets.get(sock)?)?;
                let data = mem.get(ptr..ptr + len)?;
                Some(crate::net::tcp_write(h, data) as i64)
            }
            // veil_tcp_recv(sock, ptr, cap) -> bytes read (0 empty, -1 eof/err)
            "veil_tcp_recv" => {
                let sock = *args.first()? as usize;
                let ptr = *args.get(1)? as u32 as usize;
                let cap = *args.get(2)? as u32 as usize;
                let h = (*self.sockets.get(sock)?)?;
                let mut tmp = vec![0u8; cap];
                // Wait briefly for data (the net stack runs on the same task).
                for _ in 0..2000 {
                    match crate::net::tcp_read(h, &mut tmp) {
                        crate::net::TcpRead::Data(n) => {
                            if let Some(slot) = mem.get_mut(ptr..ptr + n) {
                                slot.copy_from_slice(&tmp[..n]);
                            }
                            return Some(n as i64);
                        }
                        crate::net::TcpRead::Empty => crate::scheduler::yield_now(),
                        crate::net::TcpRead::Eof => return Some(-1),
                    }
                }
                Some(0)
            }
            // veil_tcp_close(sock)
            "veil_tcp_close" => {
                let sock = *args.first()? as usize;
                if let Some(Some(h)) = self.sockets.get(sock).copied() {
                    crate::net::tcp_close(h);
                    self.sockets[sock] = None;
                }
                Some(0)
            }
            // veil_ws_connect(url_ptr, url_len) -> ws handle (-1)
            "veil_ws_connect" => {
                if let Some(r) = self.deny(crate::perms::NETWORK) {
                    return Some(r);
                }
                let url = mem_str(mem, *args.first()?, *args.get(1)?)?;
                match crate::browser::js_ws_open(&url) {
                    Some(id) => Some(id as i64),
                    None => Some(-1),
                }
            }
            // veil_ws_send(id, msg_ptr, msg_len, out_ptr, out_cap) -> reply len
            "veil_ws_send" => {
                let id = *args.first()? as usize;
                let msg = mem_str(mem, *args.get(1)?, *args.get(2)?)?;
                let out_ptr = *args.get(3)? as u32 as usize;
                let out_cap = *args.get(4)? as u32 as usize;
                match crate::browser::js_ws_send_recv(id, &msg) {
                    Some(reply) => {
                        let b = reply.as_bytes();
                        let n = b.len().min(out_cap);
                        if let Some(slot) = mem.get_mut(out_ptr..out_ptr + n) {
                            slot.copy_from_slice(&b[..n]);
                        }
                        Some(b.len() as i64)
                    }
                    None => Some(-1),
                }
            }
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
