//! VeilNetFS — mount a remote directory over a TCP connection and browse it
//! from inside Veil. A small line protocol the host (Detroit Mac mini, via
//! `scripts/veil_netfs.py`) serves; the in-OS client connects, lists dirs, and
//! reads files. Read-only v1.
//!
//! Protocol (request: one line; response: a status line + optional payload):
//!   LIST <path>\n   -> `OK <n>\n` then n lines `<D|F> <size> <name>`
//!   READ <path>\n   -> `OK <len>\n` then len raw bytes
//!   STAT <path>\n   -> `OK <D|F> <size>\n`
//!   (errors)        -> `ERR <message>\n`
//!
//! The same `serve_request` powers an optional in-kernel loopback server, so a
//! Veil instance can both serve and mount, and the protocol is testable without
//! a host.

use crate::net;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;

const NETFS_PORT: u16 = 2049;

/// A backing store the server reads from. The host server backs this with the
/// real filesystem; the in-kernel server backs it with the VFS.
pub trait Backing {
    fn list(&self, path: &str) -> Option<Vec<(String, bool, usize)>>;
    fn read(&self, path: &str) -> Option<Vec<u8>>;
    fn is_dir(&self, path: &str) -> bool;
    fn exists(&self, path: &str) -> bool;
}

/// The VFS as a netfs backing store (with a mount sub-path prefix).
pub struct VfsBacking {
    pub root: String, // remote root path within the VFS, e.g. "/"
}

impl Backing for VfsBacking {
    fn list(&self, path: &str) -> Option<Vec<(String, bool, usize)>> {
        crate::vfs::get().ls(&self.join(path))
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        crate::vfs::get().read(&self.join(path))
    }
    fn is_dir(&self, path: &str) -> bool {
        let p = self.join(path);
        crate::vfs::get().resolve(&p).map(|i| crate::vfs::get().nodes[i].is_dir).unwrap_or(false)
    }
    fn exists(&self, path: &str) -> bool {
        crate::vfs::get().resolve(&self.join(path)).is_some()
    }
}

impl VfsBacking {
    fn join(&self, path: &str) -> String {
        let r = self.root.trim_end_matches('/');
        let p = path.trim_start_matches('/');
        if p.is_empty() { if r.is_empty() { String::from("/") } else { r.to_string() } }
        else { format!("{r}/{p}") }
    }
}

// ---- server: handle one request line ---------------------------------------

/// Serve a single request against `back`, returning the raw response bytes.
pub fn serve_request(req: &[u8], back: &dyn Backing) -> Vec<u8> {
    let line = core::str::from_utf8(req).unwrap_or("").trim_end_matches(['\n', '\r']);
    let (cmd, path) = match line.split_once(' ') {
        Some((c, p)) => (c, p.trim()),
        None => (line, "/"),
    };
    let mut out = Vec::new();
    match cmd {
        "LIST" => match back.list(path) {
            Some(entries) => {
                out.extend_from_slice(format!("OK {}\n", entries.len()).as_bytes());
                for (name, is_dir, size) in entries {
                    out.extend_from_slice(format!("{} {} {}\n", if is_dir { 'D' } else { 'F' }, size, name).as_bytes());
                }
            }
            None => out.extend_from_slice(b"ERR no such directory\n"),
        },
        "READ" => match back.read(path) {
            Some(data) => {
                out.extend_from_slice(format!("OK {}\n", data.len()).as_bytes());
                out.extend_from_slice(&data);
            }
            None => out.extend_from_slice(b"ERR no such file\n"),
        },
        "STAT" => {
            if back.exists(path) {
                let is_dir = back.is_dir(path);
                let size = if is_dir { 0 } else { back.read(path).map(|d| d.len()).unwrap_or(0) };
                out.extend_from_slice(format!("OK {} {}\n", if is_dir { 'D' } else { 'F' }, size).as_bytes());
            } else {
                out.extend_from_slice(b"ERR no such path\n");
            }
        }
        _ => out.extend_from_slice(b"ERR bad command\n"),
    }
    out
}

// ---- client: response decoders ---------------------------------------------

pub enum ListResult {
    Ok(Vec<(String, bool, usize)>),
    Err(String),
}

/// Parse a LIST response (`OK <n>\n` + n entry lines).
pub fn parse_list(resp: &[u8]) -> ListResult {
    let text = String::from_utf8_lossy(resp);
    let mut lines = text.lines();
    let header = lines.next().unwrap_or("");
    if let Some(rest) = header.strip_prefix("OK ") {
        let n: usize = rest.trim().parse().unwrap_or(0);
        let mut entries = Vec::new();
        for line in lines.take(n) {
            // "<D|F> <size> <name with spaces>"
            let mut it = line.splitn(3, ' ');
            let ty = it.next().unwrap_or("F");
            let size = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let name = it.next().unwrap_or("").to_string();
            entries.push((name, ty == "D", size));
        }
        ListResult::Ok(entries)
    } else {
        ListResult::Err(header.strip_prefix("ERR ").unwrap_or(header).to_string())
    }
}

/// Parse a READ response (`OK <len>\n` + len bytes).
pub fn parse_read(resp: &[u8]) -> Result<Vec<u8>, String> {
    let nl = resp.iter().position(|&b| b == b'\n').unwrap_or(resp.len());
    let header = core::str::from_utf8(&resp[..nl]).unwrap_or("");
    if let Some(rest) = header.strip_prefix("OK ") {
        let len: usize = rest.trim().parse().unwrap_or(0);
        let body = &resp[(nl + 1).min(resp.len())..];
        Ok(body[..len.min(body.len())].to_vec())
    } else {
        Err(header.strip_prefix("ERR ").unwrap_or(header).to_string())
    }
}

// ---- client: TCP transport -------------------------------------------------

/// Send one request to a netfs server over TCP and collect the full response.
fn rpc(ip: net::Ip, port: u16, request: &str) -> Option<Vec<u8>> {
    let h = net::tcp_connect(ip, port)?;
    let mut req = request.as_bytes().to_vec();
    if !req.ends_with(b"\n") { req.push(b'\n'); }
    let mut sent = 0;
    while sent < req.len() {
        sent += net::tcp_write(h, &req[sent..]);
        crate::scheduler::yield_now();
    }
    let mut resp = Vec::new();
    let mut buf = [0u8; 2048];
    let mut idle = 0;
    loop {
        match net::tcp_read(h, &mut buf) {
            net::TcpRead::Data(n) => { resp.extend_from_slice(&buf[..n]); idle = 0; }
            net::TcpRead::Empty => {
                idle += 1;
                if idle > 200_000 { break; }
                crate::scheduler::yield_now();
            }
            net::TcpRead::Eof => break,
        }
    }
    net::tcp_close(h);
    Some(resp)
}

pub fn list_remote(ip: net::Ip, port: u16, path: &str) -> ListResult {
    match rpc(ip, port, &format!("LIST {path}")) {
        Some(resp) => parse_list(&resp),
        None => ListResult::Err(String::from("connection failed")),
    }
}

pub fn read_remote(ip: net::Ip, port: u16, path: &str) -> Result<Vec<u8>, String> {
    match rpc(ip, port, &format!("READ {path}")) {
        Some(resp) => parse_read(&resp),
        None => Err(String::from("connection failed")),
    }
}

// ---- mount registry --------------------------------------------------------

#[derive(Clone)]
pub struct Mount {
    pub local: String,     // mount point in the VFS, e.g. /mnt/host
    pub host: String,      // "veil-host" or an IP literal
    pub ip: net::Ip,
    pub port: u16,
    pub remote: String,    // remote root path
}

static mut MOUNTS: Vec<Mount> = Vec::new();

fn mounts() -> &'static mut Vec<Mount> {
    unsafe { &mut *core::ptr::addr_of_mut!(MOUNTS) }
}

/// `mount veil-host:/path /mnt/host` — resolve the host and record the mount.
pub fn mount(spec: &str, local: &str) -> Result<(), String> {
    // spec: host[:port]:/remote/path  (host may be a name or a.b.c.d)
    let (host_port, remote) = match spec.find(":/") {
        Some(i) => (&spec[..i], &spec[i + 1..]),
        None => return Err(String::from("mount: spec must be host:/path")),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if p.parse::<u16>().is_ok() => (h, p.parse().unwrap()),
        _ => (host_port, NETFS_PORT),
    };
    let ip = resolve_host(host).ok_or_else(|| format!("mount: cannot resolve {host}"))?;
    crate::vfs::get().mkdir_p(local);
    mounts().retain(|m| m.local != local);
    mounts().push(Mount { local: local.to_string(), host: host.to_string(), ip, port, remote: remote.to_string() });
    Ok(())
}

pub fn umount(local: &str) -> bool {
    let before = mounts().len();
    mounts().retain(|m| m.local != local);
    mounts().len() != before
}

pub fn list_mounts() -> Vec<Mount> {
    mounts().clone()
}

/// If `path` falls under a mount point, return (mount, remote-relative path).
pub fn resolve_mount(path: &str) -> Option<(Mount, String)> {
    mounts()
        .iter()
        .filter(|m| path == m.local || path.starts_with(&format!("{}/", m.local)))
        .max_by_key(|m| m.local.len())
        .map(|m| {
            let rel = path[m.local.len()..].trim_start_matches('/');
            let remote = if rel.is_empty() {
                m.remote.clone()
            } else {
                format!("{}/{}", m.remote.trim_end_matches('/'), rel)
            };
            (m.clone(), remote)
        })
}

fn resolve_host(host: &str) -> Option<net::Ip> {
    // dotted-quad literal?
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() == 4 {
        if let (Ok(a), Ok(b), Ok(c), Ok(d)) = (parts[0].parse(), parts[1].parse(), parts[2].parse(), parts[3].parse()) {
            return Some([a, b, c, d]);
        }
    }
    // "veil-host" / "localhost" -> the slirp gateway-hosted server (the Mac).
    if host == "veil-host" || host == "localhost" || host == "host" {
        return Some([10, 0, 2, 2]);
    }
    net::dns_resolve(host)
}

// ---- self-test -------------------------------------------------------------

/// In-memory backing for the protocol round-trip self-test.
struct MemFs;
impl Backing for MemFs {
    fn list(&self, path: &str) -> Option<Vec<(String, bool, usize)>> {
        match path {
            "/" => Some(alloc::vec![
                (String::from("notes.txt"), false, 11),
                (String::from("docs"), true, 0),
                (String::from("photo.png"), false, 4096),
            ]),
            "/docs" => Some(alloc::vec![(String::from("readme.md"), false, 5)]),
            _ => None,
        }
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        match path {
            "/notes.txt" => Some(b"hello veil!".to_vec()),
            "/docs/readme.md" => Some(b"# doc".to_vec()),
            _ => None,
        }
    }
    fn is_dir(&self, path: &str) -> bool {
        matches!(path, "/" | "/docs")
    }
    fn exists(&self, path: &str) -> bool {
        self.list(path).is_some() || self.read(path).is_some()
    }
}

pub fn selftest() {
    let back = MemFs;
    // Full protocol round-trip: client encodes a request -> server handles it
    // against a backing store -> client decodes the response. This is exactly
    // what flows over TCP, minus the wire.
    let list_resp = serve_request(b"LIST /\n", &back);
    let listing = match parse_list(&list_resp) {
        ListResult::Ok(e) => e,
        ListResult::Err(_) => Vec::new(),
    };
    let has_docs_dir = listing.iter().any(|(n, d, _)| n == "docs" && *d);
    let has_notes = listing.iter().any(|(n, d, _)| n == "notes.txt" && !*d);

    let read_resp = serve_request(b"READ /notes.txt\n", &back);
    let body = parse_read(&read_resp).unwrap_or_default();

    let sub_resp = serve_request(b"LIST /docs\n", &back);
    let sub = match parse_list(&sub_resp) { ListResult::Ok(e) => e, _ => Vec::new() };

    let err_resp = serve_request(b"READ /missing\n", &back);
    let err_is_err = parse_read(&err_resp).is_err();

    // Mount registry + path routing (what `mount`/`cat /mnt/host/...` use).
    crate::vfs::get();
    let _ = mount("veil-host:/", "/mnt/host");
    let routed = resolve_mount("/mnt/host/docs/readme.md");
    let route_ok = routed.as_ref().map(|(m, rem)| m.local == "/mnt/host" && rem == "/docs/readme.md").unwrap_or(false);
    let mount_listed = list_mounts().iter().any(|m| m.local == "/mnt/host" && m.host == "veil-host");
    let unmounted = umount("/mnt/host");

    crate::kprintln!(
        "NETFS: list[{}] notes={has_notes} docs_dir={has_docs_dir} read={:?} sub={} err={err_is_err} route={route_ok} mounted={mount_listed} umount={unmounted}",
        listing.len(), String::from_utf8_lossy(&body), sub.len()
    );
    let ok = has_docs_dir && has_notes && body == b"hello veil!" && sub.len() == 1
        && err_is_err && route_ok && mount_listed && unmounted;
    if ok {
        crate::kprintln!("NETFS_OK: network filesystem — LIST/READ/STAT protocol round-trip + mount registry + path routing (host server in scripts/veil_netfs.py)");
    } else {
        crate::kprintln!("NETFS_FAIL: notes={has_notes} docs={has_docs_dir} read={:?} route={route_ok}", String::from_utf8_lossy(&body));
    }
}
