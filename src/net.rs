//! The protocol stack: Ethernet demux, ARP, IPv4, ICMP echo, a UDP echo
//! service, and TCP (M12-M14).
//!
//! Everything runs under masked IRQs: `on_frame` is called from the
//! virtio-net IRQ handler, the socket API (used by the HTTP/echo task)
//! wraps itself in `critical`, and `on_tick` runs from the timer IRQ for
//! retransmission. Single core, so masking IRQs is the whole story.
//!
//! TCP scope: server-side (LISTEN -> accept) plus active open
//! (connect, M16's browser), in-order receive only (out-of-order
//! segments are dropped and dup-ACKed; the peer retransmits), go-back-N
//! retransmission with exponential backoff. No window scaling, no SACK,
//! no congestion control — correct and simple beats fast on a LAN/NAT
//! with ~zero loss.
//!
//! Loopback: packets addressed to our own IP never touch the wire; they
//! go on a queue drained iteratively after every entry point, so the
//! on-OS browser talks to the on-OS HTTP server through the same TCP
//! state machine, both directions ours.

use crate::{kprintln, netdev};
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const ETH_IP: u16 = 0x0800;
const ETH_ARP: u16 = 0x0806;
const PROTO_ICMP: u8 = 1;
const PROTO_TCP: u8 = 6;
const PROTO_UDP: u8 = 17;

const MSS: usize = 1460;
const RECV_CAP: usize = 32 * 1024;
pub const SEND_CAP: usize = 32 * 1024;
const RTO_TICKS: u32 = 10; // 200 ms at the 50 Hz tick
const MAX_RETRIES: u32 = 8;
const TIME_WAIT_TICKS: u32 = 100; // 2 s
const UDP_ECHO_PORT: u16 = 7;

const FLAG_FIN: u8 = 0x01;
const FLAG_SYN: u8 = 0x02;
const FLAG_RST: u8 = 0x04;
const FLAG_PSH: u8 = 0x08;
const FLAG_ACK: u8 = 0x10;

type Ip = [u8; 4];

pub fn fmt_mac(m: &[u8; 6]) -> String {
    let mut s = String::new();
    let _ = write!(s, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5]);
    s
}

pub fn fmt_ip(ip: &Ip) -> String {
    let mut s = String::new();
    let _ = write!(s, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    s
}

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

// --- TCP connection state ----------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum TcpState {
    SynSent, // active open: our SYN is out
    SynRcvd,
    Established,
    FinWait1, // we sent FIN, unacked
    FinWait2, // our FIN acked, waiting for theirs
    Closing,  // both FINs in flight, ours unacked
    TimeWait,
    CloseWait, // they closed, app still open
    LastAck,   // they closed, we sent FIN
    Closed,
}

struct Conn {
    tag: u32,
    state: TcpState,
    remote_ip: Ip,
    remote_port: u16,
    local_port: u16,
    iss: u32,
    snd_una: u32, // oldest unacked seq; sendq[0] sits here
    snd_nxt: u32,
    rcv_nxt: u32,
    peer_win: u32,
    peer_mss: usize,
    sendq: Vec<u8>, // unacked + unsent bytes
    recvq: Vec<u8>,
    fin_sent: bool,
    fin_acked: bool,
    app_closed: bool,
    accepted: bool,
    rto_left: u32, // 0 = timer off
    retries: u32,
    time_wait_left: u32,
    rx_bytes: usize,
    tx_bytes: usize,
}

struct State {
    ip: Ip,
    prefix: u32,
    gw: Ip,
    mac: [u8; 6],
    arp: Vec<(Ip, [u8; 6])>,
    listeners: Vec<u16>,
    conns: Vec<Option<Conn>>,
    next_gen: u32,
    ip_ident: u16,
    next_port: u16,         // ephemeral port allocator for connect()
    loopback: Vec<Vec<u8>>, // IP packets to ourselves, pending re-input
    chat_rx: Vec<Vec<u8>>,  // M20: datagrams received on udp :7777
    udp_rx: Vec<(u16, Vec<u8>)>, // client datagrams by local port (DNS/NTP)
}

/// M20 chat port (UDP; the TCP echo on the same number is a different
/// namespace) and the limited-broadcast address chat sends to.
const CHAT_PORT: u16 = 7777;
const BROADCAST: Ip = [255, 255, 255, 255];

/// Deliver any queued to-ourselves packets. Iterative (the queue may grow
/// while we drain it), so a loopback handshake never recurses.
fn pump_loopback(st: &mut State) {
    let mut budget = 10_000; // backstop against a livelocked exchange
    while !st.loopback.is_empty() && budget > 0 {
        let pkt = st.loopback.remove(0);
        ip_input(st, &pkt);
        budget -= 1;
    }
}

static mut STATE: Option<State> = None;
static RX_FRAMES: AtomicU64 = AtomicU64::new(0);
static FIRST_DUMP: AtomicBool = AtomicBool::new(false);
static ICMP_OK: AtomicBool = AtomicBool::new(false);
static TCP_CLOSE_OK: AtomicBool = AtomicBool::new(false);

fn state() -> Option<&'static mut State> {
    unsafe { (*core::ptr::addr_of_mut!(STATE)).as_mut() }
}

pub fn rx_count() -> u64 {
    RX_FRAMES.load(Ordering::Relaxed)
}

/// Parse "a.b.c.d/len,gateway" (the opt/veil.net fw_cfg string).
pub fn parse_config(s: &str) -> Option<(Ip, u32, Ip)> {
    let (addr, gw) = s.trim().split_once(',')?;
    let (ip, prefix) = addr.split_once('/')?;
    let parse_ip = |t: &str| -> Option<Ip> {
        let mut out = [0u8; 4];
        let mut it = t.split('.');
        for b in out.iter_mut() {
            *b = it.next()?.parse().ok()?;
        }
        it.next().is_none().then_some(out)
    };
    Some((parse_ip(ip)?, prefix.parse().ok()?, parse_ip(gw)?))
}

pub fn init(mac: [u8; 6], ip: Ip, prefix: u32, gw: Ip) {
    critical(|| unsafe {
        *core::ptr::addr_of_mut!(STATE) = Some(State {
            ip,
            prefix,
            gw,
            mac,
            arp: Vec::new(),
            listeners: Vec::new(),
            conns: Vec::new(),
            next_gen: 1,
            ip_ident: 1,
            next_port: 49152,
            loopback: Vec::new(),
            chat_rx: Vec::new(),
            udp_rx: Vec::new(),
        });
    });
    kprintln!(
        "NETSTACK: ip {}/{prefix}, gw {} (arp, ipv4, icmp echo, udp echo :7, tcp)",
        fmt_ip(&ip),
        fmt_ip(&gw)
    );
}

// --- checksums ---------------------------------------------------------------

fn csum_add(mut sum: u32, data: &[u8]) -> u32 {
    let mut chunks = data.chunks_exact(2);
    for c in &mut chunks {
        sum += u16::from_be_bytes([c[0], c[1]]) as u32;
    }
    if let [last] = chunks.remainder() {
        sum += (*last as u32) << 8;
    }
    sum
}

fn csum_fin(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Pseudo-header sum for TCP/UDP.
fn csum_pseudo(src: &Ip, dst: &Ip, proto: u8, len: usize) -> u32 {
    let mut sum = csum_add(0, src);
    sum = csum_add(sum, dst);
    sum + proto as u32 + len as u32
}

// --- frame input (IRQ context) -------------------------------------------------

pub fn on_frame(frame: &[u8]) {
    let n = RX_FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 && !FIRST_DUMP.swap(true, Ordering::Relaxed) {
        // M12 proof: print the bytes of a frame we received.
        let mut hex = String::new();
        for b in frame.iter().take(48) {
            let _ = write!(hex, "{b:02x} ");
        }
        kprintln!("NET_RX: {} bytes: {}{}", frame.len(), hex.trim_end(),
                  if frame.len() > 48 { " ..." } else { "" });
    }
    if frame.len() < 14 {
        return;
    }
    let Some(st) = state() else { return };
    // Ignore frames we sent ourselves. A multicast/hub bridge (M20's
    // two-instance LAN) loops a sender's own broadcasts back to it; without
    // this a node would receive and render its own chat messages twice.
    // Genuine on-OS loopback never touches the wire (it uses the internal
    // queue), so a self-MAC source here is always an external echo.
    if frame[6..12] == st.mac {
        return;
    }
    match u16::from_be_bytes([frame[12], frame[13]]) {
        ETH_ARP => arp_input(st, &frame[14..]),
        ETH_IP => ip_input(st, &frame[14..]),
        _ => {}
    }
    pump_loopback(st);
}

fn arp_cache(st: &mut State, ip: Ip, mac: [u8; 6]) {
    if ip == [0; 4] {
        return;
    }
    match st.arp.iter_mut().find(|(i, _)| *i == ip) {
        Some(e) => e.1 = mac,
        None => st.arp.push((ip, mac)),
    }
}

fn arp_input(st: &mut State, p: &[u8]) {
    if p.len() < 28 || p[0..2] != [0, 1] || p[2..4] != [8, 0] || p[4] != 6 || p[5] != 4 {
        return;
    }
    let oper = u16::from_be_bytes([p[6], p[7]]);
    let sha: [u8; 6] = p[8..14].try_into().unwrap();
    let spa: Ip = p[14..18].try_into().unwrap();
    let tpa: Ip = p[24..28].try_into().unwrap();
    arp_cache(st, spa, sha);
    if oper == 1 && tpa == st.ip {
        let mut reply = [0u8; 28];
        reply[0..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
        reply[8..14].copy_from_slice(&st.mac);
        reply[14..18].copy_from_slice(&st.ip);
        reply[18..24].copy_from_slice(&sha);
        reply[24..28].copy_from_slice(&spa);
        eth_send(st, &sha, ETH_ARP, &reply);
        kprintln!("ARP: who-has {} from {} -> replied", fmt_ip(&st.ip), fmt_ip(&spa));
    }
}

/// Broadcast an ARP request (also the M12 "make something talk to us" probe).
pub fn arp_probe(target: Ip) {
    critical(|| {
        let Some(st) = state() else { return };
        let mut req = [0u8; 28];
        req[0..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        req[8..14].copy_from_slice(&st.mac);
        req[14..18].copy_from_slice(&st.ip);
        req[24..28].copy_from_slice(&target);
        eth_send(st, &[0xff; 6], ETH_ARP, &req);
    });
}

fn ip_input(st: &mut State, p: &[u8]) {
    if p.len() < 20 || p[0] >> 4 != 4 {
        return;
    }
    let ihl = (p[0] & 0xf) as usize * 4;
    let total = u16::from_be_bytes([p[2], p[3]]) as usize;
    if ihl < 20 || total < ihl || total > p.len() {
        return;
    }
    let frag = u16::from_be_bytes([p[6], p[7]]);
    if frag & 0x3fff != 0 {
        return; // fragmented: out of scope, peer MTU matches ours
    }
    let src: Ip = p[12..16].try_into().unwrap();
    let dst: Ip = p[16..20].try_into().unwrap();
    if dst != st.ip && dst != BROADCAST {
        return; // (chat broadcasts are the only broadcast we consume)
    }
    let payload = &p[ihl..total];
    match p[9] {
        PROTO_ICMP => icmp_input(st, src, payload),
        PROTO_UDP => udp_input(st, src, payload),
        PROTO_TCP => tcp_input(st, src, payload),
        _ => {}
    }
}

fn icmp_input(st: &mut State, src: Ip, p: &[u8]) {
    if p.len() < 8 || p[0] != 8 || p[1] != 0 {
        return; // only echo requests
    }
    let mut reply = Vec::with_capacity(p.len());
    reply.extend_from_slice(p);
    reply[0] = 0; // echo reply
    reply[2] = 0;
    reply[3] = 0;
    let sum = csum_fin(csum_add(0, &reply)).to_be_bytes();
    reply[2..4].copy_from_slice(&sum);
    let seq = u16::from_be_bytes([p[6], p[7]]);
    ip_output(st, PROTO_ICMP, src, &reply);
    kprintln!("ICMP: echo {} bytes from {} seq={seq} -> reply", p.len(), fmt_ip(&src));
    if !ICMP_OK.swap(true, Ordering::Relaxed) {
        kprintln!("ICMP_OK: answered a ping");
        kprintln!("M13_OK");
    }
}

fn udp_input(st: &mut State, src: Ip, p: &[u8]) {
    if p.len() < 8 {
        return;
    }
    let sport = u16::from_be_bytes([p[0], p[1]]);
    let dport = u16::from_be_bytes([p[2], p[3]]);
    let len = (u16::from_be_bytes([p[4], p[5]]) as usize).min(p.len());
    if dport == CHAT_PORT && len >= 8 {
        // M20: queue for the chat window (drained by the desktop loop).
        if st.chat_rx.len() < 64 {
            st.chat_rx.push(p[8..len.min(8 + 128)].to_vec());
        }
        return;
    }
    if (49152..61000).contains(&dport) && len >= 8 {
        // Client traffic (DNS / NTP replies) — queue by local port.
        if st.udp_rx.len() < 16 {
            st.udp_rx.push((dport, p[8..len].to_vec()));
        }
        return;
    }
    if dport != UDP_ECHO_PORT || len < 8 {
        return;
    }
    let payload = &p[8..len];
    let mut dgram = Vec::with_capacity(8 + payload.len());
    dgram.extend_from_slice(&UDP_ECHO_PORT.to_be_bytes());
    dgram.extend_from_slice(&sport.to_be_bytes());
    dgram.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    dgram.extend_from_slice(&[0, 0]);
    dgram.extend_from_slice(payload);
    let sum = csum_fin(csum_add(csum_pseudo(&st.ip, &src, PROTO_UDP, dgram.len()), &dgram));
    let sum = if sum == 0 { 0xffff } else { sum };
    dgram[6..8].copy_from_slice(&sum.to_be_bytes());
    ip_output(st, PROTO_UDP, src, &dgram);
    kprintln!("UDP: echoed {} bytes to {}:{sport}", payload.len(), fmt_ip(&src));
}

// --- output path ---------------------------------------------------------------

fn eth_send(st: &State, dst: &[u8; 6], ethertype: u16, payload: &[u8]) {
    let mut frame = [0u8; 1514];
    let len = 14 + payload.len();
    frame[0..6].copy_from_slice(dst);
    frame[6..12].copy_from_slice(&st.mac);
    frame[12..14].copy_from_slice(&ethertype.to_be_bytes());
    frame[14..len].copy_from_slice(payload);
    netdev::send(&frame[..len.max(60)]); // pad runts to minimum frame size
}

/// Route + emit one IPv4 packet. Returns false on an ARP miss (a request
/// was sent; TCP's retransmit or the peer's retry recovers). Packets to
/// our own address skip the wire and queue for loopback delivery.
fn ip_output(st: &mut State, proto: u8, dst: Ip, payload: &[u8]) -> bool {
    if dst == st.ip {
        let mut pkt = Vec::with_capacity(20 + payload.len());
        pkt.extend_from_slice(&[0x45, 0]);
        pkt.extend_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
        pkt.extend_from_slice(&st.ip_ident.to_be_bytes());
        st.ip_ident = st.ip_ident.wrapping_add(1);
        pkt.extend_from_slice(&[0x40, 0, 64, proto, 0, 0]);
        pkt.extend_from_slice(&st.ip);
        pkt.extend_from_slice(&dst);
        pkt.extend_from_slice(payload);
        st.loopback.push(pkt);
        return true;
    }
    if dst == BROADCAST {
        // Limited broadcast: no route, no ARP — straight to ff:ff:ff:ff:ff:ff.
        let mut pkt = Vec::with_capacity(20 + payload.len());
        pkt.extend_from_slice(&[0x45, 0]);
        pkt.extend_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
        pkt.extend_from_slice(&st.ip_ident.to_be_bytes());
        st.ip_ident = st.ip_ident.wrapping_add(1);
        pkt.extend_from_slice(&[0x40, 0, 64, proto, 0, 0]);
        pkt.extend_from_slice(&st.ip);
        pkt.extend_from_slice(&dst);
        let sum = csum_fin(csum_add(0, &pkt)).to_be_bytes();
        pkt[10..12].copy_from_slice(&sum);
        pkt.extend_from_slice(payload);
        eth_send(st, &[0xff; 6], ETH_IP, &pkt);
        return true;
    }
    let mask = u32::MAX.checked_shl(32 - st.prefix).unwrap_or(0);
    let on_link = (u32::from_be_bytes(dst) ^ u32::from_be_bytes(st.ip)) & mask == 0;
    let hop = if on_link { dst } else { st.gw };
    let Some(&(_, mac)) = st.arp.iter().find(|(i, _)| *i == hop) else {
        let mut req = [0u8; 28];
        req[0..8].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        req[8..14].copy_from_slice(&st.mac);
        req[14..18].copy_from_slice(&st.ip);
        req[24..28].copy_from_slice(&hop);
        eth_send(st, &[0xff; 6], ETH_ARP, &req);
        return false;
    };

    let mut pkt = Vec::with_capacity(20 + payload.len());
    pkt.extend_from_slice(&[0x45, 0]);
    pkt.extend_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
    pkt.extend_from_slice(&st.ip_ident.to_be_bytes());
    st.ip_ident = st.ip_ident.wrapping_add(1);
    pkt.extend_from_slice(&[0x40, 0, 64, proto, 0, 0]); // DF, ttl 64
    pkt.extend_from_slice(&st.ip);
    pkt.extend_from_slice(&dst);
    let sum = csum_fin(csum_add(0, &pkt)).to_be_bytes();
    pkt[10..12].copy_from_slice(&sum);
    pkt.extend_from_slice(payload);
    eth_send(st, &mac, ETH_IP, &pkt);
    true
}

// --- TCP -------------------------------------------------------------------------

fn seq_lt(a: u32, b: u32) -> bool {
    (a.wrapping_sub(b) as i32) < 0
}

fn seq_le(a: u32, b: u32) -> bool {
    !seq_lt(b, a)
}

fn isn() -> u32 {
    let cnt: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) cnt, options(nomem, nostack)) };
    (cnt >> 6) as u32 // ~1MHz-ish tick: monotone-enough ISNs
}

fn our_window(c: &Conn) -> u16 {
    (RECV_CAP - c.recvq.len()).min(0xffff) as u16
}

/// Emit one segment for `c`. `payload` must already fit the peer's MSS.
/// `flags` is sent verbatim — callers include FLAG_ACK themselves (the
/// only segment without it is an active open's first SYN).
fn send_segment(st: &mut State, ci: usize, seq: u32, flags: u8, payload: &[u8]) {
    let c = st.conns[ci].as_ref().unwrap();
    let (remote_ip, rport, lport) = (c.remote_ip, c.remote_port, c.local_port);
    let (ack, win) = (c.rcv_nxt, our_window(c));
    let with_mss = flags & FLAG_SYN != 0;
    let doff: u8 = if with_mss { 6 } else { 5 };

    let mut seg = Vec::with_capacity(doff as usize * 4 + payload.len());
    seg.extend_from_slice(&lport.to_be_bytes());
    seg.extend_from_slice(&rport.to_be_bytes());
    seg.extend_from_slice(&seq.to_be_bytes());
    seg.extend_from_slice(&ack.to_be_bytes());
    seg.push(doff << 4);
    seg.push(flags);
    seg.extend_from_slice(&win.to_be_bytes());
    seg.extend_from_slice(&[0, 0, 0, 0]); // checksum, urgent
    if with_mss {
        seg.extend_from_slice(&[2, 4]);
        seg.extend_from_slice(&(MSS as u16).to_be_bytes());
    }
    seg.extend_from_slice(payload);
    let sum = csum_fin(csum_add(csum_pseudo(&st.ip, &remote_ip, PROTO_TCP, seg.len()), &seg));
    seg[16..18].copy_from_slice(&sum.to_be_bytes());
    ip_output(st, PROTO_TCP, remote_ip, &seg);
}

fn send_rst(st: &mut State, dst: Ip, sport: u16, dport: u16, seq: u32, ack: u32) {
    let mut seg = Vec::with_capacity(20);
    seg.extend_from_slice(&dport.to_be_bytes());
    seg.extend_from_slice(&sport.to_be_bytes());
    seg.extend_from_slice(&seq.to_be_bytes());
    seg.extend_from_slice(&ack.to_be_bytes());
    seg.extend_from_slice(&[5 << 4, FLAG_RST | FLAG_ACK, 0, 0, 0, 0, 0, 0]);
    let sum = csum_fin(csum_add(csum_pseudo(&st.ip, &dst, PROTO_TCP, seg.len()), &seg));
    seg[16..18].copy_from_slice(&sum.to_be_bytes());
    ip_output(st, PROTO_TCP, dst, &seg);
}

/// Push out whatever `c` may send: data within the peer's window, then a
/// FIN once the app has closed and the sendq is drained. Returns true if
/// any segment (which always carries an ACK) went out.
fn tcp_output(st: &mut State, ci: usize) -> bool {
    let mut sent = false;
    loop {
        let c = st.conns[ci].as_ref().unwrap();
        if c.fin_sent || c.state == TcpState::SynRcvd {
            break;
        }
        let inflight = c.snd_nxt.wrapping_sub(c.snd_una) as usize;
        if inflight < c.sendq.len() {
            let win = (c.peer_win as usize).max(1); // probe a zero window
            if inflight >= win {
                break;
            }
            let n = (c.sendq.len() - inflight).min(c.peer_mss).min(win - inflight);
            let payload: Vec<u8> = c.sendq[inflight..inflight + n].to_vec();
            let seq = c.snd_nxt;
            send_segment(st, ci, seq, FLAG_PSH | FLAG_ACK, &payload);
            let c = st.conns[ci].as_mut().unwrap();
            c.snd_nxt = c.snd_nxt.wrapping_add(n as u32);
            c.tx_bytes += n;
            if c.rto_left == 0 {
                c.rto_left = RTO_TICKS;
            }
            sent = true;
        } else if c.app_closed {
            let seq = c.snd_nxt;
            send_segment(st, ci, seq, FLAG_FIN | FLAG_ACK, &[]);
            let c = st.conns[ci].as_mut().unwrap();
            c.fin_sent = true;
            c.snd_nxt = c.snd_nxt.wrapping_add(1);
            c.state = match c.state {
                TcpState::CloseWait => TcpState::LastAck,
                _ => TcpState::FinWait1,
            };
            if c.rto_left == 0 {
                c.rto_left = RTO_TICKS;
            }
            sent = true;
            break;
        } else {
            break;
        }
    }
    sent
}

fn log_clean_close(c: &Conn) {
    kprintln!(
        "TCP: clean close {}:{} <-> :{} (rx {} tx {} bytes)",
        fmt_ip(&c.remote_ip),
        c.remote_port,
        c.local_port,
        c.rx_bytes,
        c.tx_bytes
    );
    if c.rx_bytes > 0 && c.tx_bytes > 0 && !TCP_CLOSE_OK.swap(true, Ordering::Relaxed) {
        kprintln!("TCP_OK: handshake, two-way data, orderly FIN teardown");
        kprintln!("M14_OK");
    }
}

/// The MSS option from a SYN's option block, or the RFC default.
fn parse_mss(p: &[u8], doff: usize) -> usize {
    let mut mss = 536;
    let mut o = 20;
    while o < doff && p[o] != 0 {
        match p[o] {
            1 => o += 1,
            2 if o + 4 <= doff => {
                mss = u16::from_be_bytes([p[o + 2], p[o + 3]]) as usize;
                o += 4;
            }
            _ if o + 1 < doff && p[o + 1] >= 2 => o += p[o + 1] as usize,
            _ => break,
        }
    }
    mss
}

fn tcp_input(st: &mut State, src: Ip, p: &[u8]) {
    if p.len() < 20 {
        return;
    }
    let sport = u16::from_be_bytes([p[0], p[1]]);
    let dport = u16::from_be_bytes([p[2], p[3]]);
    let seq = u32::from_be_bytes([p[4], p[5], p[6], p[7]]);
    let ack = u32::from_be_bytes([p[8], p[9], p[10], p[11]]);
    let doff = (p[12] >> 4) as usize * 4;
    let flags = p[13];
    let win = u16::from_be_bytes([p[14], p[15]]) as u32;
    if doff < 20 || doff > p.len() {
        return;
    }
    let payload = &p[doff..];

    let ci = st.conns.iter().position(|c| {
        c.as_ref().is_some_and(|c| {
            c.state != TcpState::Closed
                && c.remote_ip == src
                && c.remote_port == sport
                && c.local_port == dport
        })
    });

    let Some(ci) = ci else {
        if flags & FLAG_RST != 0 {
            return;
        }
        if flags & FLAG_SYN != 0 && flags & FLAG_ACK == 0 && st.listeners.contains(&dport) {
            let peer_mss = parse_mss(p, doff);
            let iss = isn();
            let conn = Conn {
                tag: st.next_gen,
                state: TcpState::SynRcvd,
                remote_ip: src,
                remote_port: sport,
                local_port: dport,
                iss,
                snd_una: iss,
                snd_nxt: iss.wrapping_add(1),
                rcv_nxt: seq.wrapping_add(1),
                peer_win: win,
                peer_mss: peer_mss.clamp(536, MSS),
                sendq: Vec::new(),
                recvq: Vec::new(),
                fin_sent: false,
                fin_acked: false,
                app_closed: false,
                accepted: false,
                rto_left: RTO_TICKS,
                retries: 0,
                time_wait_left: 0,
                rx_bytes: 0,
                tx_bytes: 0,
            };
            st.next_gen += 1;
            let ci = match st.conns.iter().position(|c| c.is_none()) {
                Some(i) => {
                    st.conns[i] = Some(conn);
                    i
                }
                None => {
                    st.conns.push(Some(conn));
                    st.conns.len() - 1
                }
            };
            send_segment(st, ci, iss, FLAG_SYN | FLAG_ACK, &[]);
        } else {
            // Nothing here: tell the peer so it fails fast instead of retrying.
            let rst_seq = if flags & FLAG_ACK != 0 { ack } else { 0 };
            send_rst(st, src, sport, dport, rst_seq, seq.wrapping_add(payload.len() as u32));
        }
        return;
    };

    // --- existing connection ---
    if flags & FLAG_RST != 0 {
        let c = st.conns[ci].as_mut().unwrap();
        kprintln!("TCP: reset by {}:{}", fmt_ip(&c.remote_ip), c.remote_port);
        c.state = TcpState::Closed;
        return;
    }

    // Duplicate SYN (our SYN-ACK got lost): resend it.
    if flags & FLAG_SYN != 0 && st.conns[ci].as_ref().unwrap().state == TcpState::SynRcvd {
        let iss = st.conns[ci].as_ref().unwrap().iss;
        send_segment(st, ci, iss, FLAG_SYN | FLAG_ACK, &[]);
        return;
    }

    // Active open: the SYN-ACK answering our SYN completes the handshake.
    if st.conns[ci].as_ref().unwrap().state == TcpState::SynSent {
        if flags & (FLAG_SYN | FLAG_ACK) == FLAG_SYN | FLAG_ACK {
            let c = st.conns[ci].as_mut().unwrap();
            if ack != c.snd_nxt {
                return; // not for our SYN; let the peer retransmit
            }
            c.rcv_nxt = seq.wrapping_add(1);
            c.snd_una = ack;
            c.peer_win = win;
            c.peer_mss = parse_mss(p, doff).clamp(536, MSS);
            c.state = TcpState::Established;
            c.rto_left = 0;
            c.retries = 0;
            kprintln!(
                "TCP: connected :{} -> {}:{}",
                c.local_port,
                fmt_ip(&c.remote_ip),
                c.remote_port
            );
            let seq_out = c.snd_nxt;
            if !tcp_output(st, ci) {
                send_segment(st, ci, seq_out, FLAG_ACK, &[]);
            }
        }
        return; // anything else in SynSent: ignore, retransmit recovers
    }

    let mut ack_needed = false;
    let mut close_to_log = false;

    if flags & FLAG_ACK != 0 {
        let c = st.conns[ci].as_mut().unwrap();
        c.peer_win = win;
        if c.state == TcpState::SynRcvd && ack == c.snd_nxt {
            c.state = TcpState::Established;
            kprintln!(
                "TCP: established {}:{} -> :{}",
                fmt_ip(&c.remote_ip),
                c.remote_port,
                c.local_port
            );
        }
        if seq_lt(c.snd_una, ack) && seq_le(ack, c.snd_nxt) {
            let advanced = ack.wrapping_sub(c.snd_una) as usize;
            let data_acked = advanced.min(c.sendq.len());
            c.sendq.drain(..data_acked);
            if advanced > data_acked && c.fin_sent {
                c.fin_acked = true;
                match c.state {
                    TcpState::FinWait1 => c.state = TcpState::FinWait2,
                    TcpState::Closing => {
                        c.state = TcpState::TimeWait;
                        c.time_wait_left = TIME_WAIT_TICKS;
                        close_to_log = true;
                    }
                    TcpState::LastAck => {
                        c.state = TcpState::Closed;
                        close_to_log = true;
                    }
                    _ => {}
                }
            }
            c.snd_una = ack;
            c.retries = 0;
            c.rto_left = if c.snd_una == c.snd_nxt { 0 } else { RTO_TICKS };
        }
    }

    // In-order data only; anything else is dropped and dup-ACKed.
    let mut fin_in_order = false;
    {
        let c = st.conns[ci].as_mut().unwrap();
        let mut seg_seq = seq;
        if !payload.is_empty() {
            if seg_seq == c.rcv_nxt && c.recvq.len() + payload.len() <= RECV_CAP {
                c.recvq.extend_from_slice(payload);
                c.rcv_nxt = c.rcv_nxt.wrapping_add(payload.len() as u32);
                c.rx_bytes += payload.len();
                seg_seq = c.rcv_nxt;
            } else {
                seg_seq = u32::MAX; // poison: the FIN below is out of order too
            }
            ack_needed = true;
        }
        if flags & FLAG_FIN != 0 {
            if seg_seq == c.rcv_nxt {
                c.rcv_nxt = c.rcv_nxt.wrapping_add(1);
                fin_in_order = true;
            }
            ack_needed = true;
        }
    }
    if fin_in_order {
        let c = st.conns[ci].as_mut().unwrap();
        match c.state {
            TcpState::Established | TcpState::SynRcvd => {
                c.state = TcpState::CloseWait;
                kprintln!("TCP: peer {}:{} closed (half)", fmt_ip(&c.remote_ip), c.remote_port);
            }
            TcpState::FinWait1 => c.state = TcpState::Closing,
            TcpState::FinWait2 => {
                c.state = TcpState::TimeWait;
                c.time_wait_left = TIME_WAIT_TICKS;
                close_to_log = true;
            }
            _ => {}
        }
    }

    if close_to_log {
        log_clean_close(st.conns[ci].as_ref().unwrap());
    }

    let sent = tcp_output(st, ci);
    if ack_needed && !sent {
        let seq = st.conns[ci].as_ref().unwrap().snd_nxt;
        send_segment(st, ci, seq, FLAG_ACK, &[]);
    }

    // Fully closed and nobody will touch it again: reap the slot. A conn
    // the app accepted but hasn't released stays until tcp_close().
    let c = st.conns[ci].as_ref().unwrap();
    if c.state == TcpState::Closed && (!c.accepted || c.app_closed) {
        st.conns[ci] = None;
    }
}

/// 50 Hz housekeeping from the timer IRQ: retransmission and timers.
pub fn on_tick() {
    let Some(st) = state() else { return };
    for ci in 0..st.conns.len() {
        let Some(c) = st.conns[ci].as_mut() else { continue };
        if c.state == TcpState::TimeWait {
            c.time_wait_left = c.time_wait_left.saturating_sub(1);
            if c.time_wait_left == 0 {
                c.state = TcpState::Closed;
                if !c.accepted || c.app_closed {
                    st.conns[ci] = None;
                }
            }
            continue;
        }
        if c.rto_left == 0 || c.state == TcpState::Closed {
            continue;
        }
        c.rto_left -= 1;
        if c.rto_left > 0 {
            continue;
        }
        c.retries += 1;
        if c.retries > MAX_RETRIES {
            kprintln!(
                "TCP: retransmit limit, dropping {}:{}",
                fmt_ip(&c.remote_ip),
                c.remote_port
            );
            c.state = TcpState::Closed;
            if !c.accepted || c.app_closed {
                st.conns[ci] = None;
            }
            continue;
        }
        c.rto_left = RTO_TICKS << c.retries.min(5);
        let (state_now, iss, snd_una, fin_only, n) = (
            c.state,
            c.iss,
            c.snd_una,
            c.sendq.is_empty() && c.fin_sent && !c.fin_acked,
            c.sendq.len().min(c.peer_mss),
        );
        if state_now == TcpState::SynSent {
            send_segment(st, ci, iss, FLAG_SYN, &[]);
        } else if state_now == TcpState::SynRcvd {
            send_segment(st, ci, iss, FLAG_SYN | FLAG_ACK, &[]);
        } else if n > 0 {
            let payload: Vec<u8> = st.conns[ci].as_ref().unwrap().sendq[..n].to_vec();
            send_segment(st, ci, snd_una, FLAG_PSH | FLAG_ACK, &payload);
        } else if fin_only {
            send_segment(st, ci, snd_una, FLAG_FIN | FLAG_ACK, &[]);
        }
    }
    pump_loopback(st);
}

// --- socket API (task context) -------------------------------------------------

#[derive(Clone, Copy)]
pub struct Handle {
    idx: usize,
    tag: u32,
}

pub enum TcpRead {
    Data(usize),
    Empty,
    Eof,
}

pub fn tcp_listen(port: u16) {
    critical(|| {
        if let Some(st) = state() {
            if !st.listeners.contains(&port) {
                st.listeners.push(port);
            }
        }
    });
}

pub fn tcp_listening(port: u16) -> bool {
    critical(|| state().is_some_and(|st| st.listeners.contains(&port)))
}

pub fn local_ip() -> Option<Ip> {
    critical(|| state().map(|st| st.ip))
}

/// Build + checksum + route one UDP datagram (locked context).
fn udp_emit(st: &mut State, dst: Ip, dport: u16, sport: u16, payload: &[u8]) -> bool {
    let mut dgram = Vec::with_capacity(8 + payload.len());
    dgram.extend_from_slice(&sport.to_be_bytes());
    dgram.extend_from_slice(&dport.to_be_bytes());
    dgram.extend_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    dgram.extend_from_slice(&[0, 0]);
    dgram.extend_from_slice(payload);
    let sum = csum_fin(csum_add(
        csum_pseudo(&st.ip, &dst, PROTO_UDP, dgram.len()),
        &dgram,
    ));
    let sum = if sum == 0 { 0xffff } else { sum };
    dgram[6..8].copy_from_slice(&sum.to_be_bytes());
    ip_output(st, PROTO_UDP, dst, &dgram)
}

/// M20: broadcast one chat datagram (udp :7777 -> :7777, limited
/// broadcast). The QEMU `-netdev socket` bridge carries it to the peer
/// instance; our own stack never sees it back (no self-delivery).
pub fn chat_send(payload: &[u8]) -> bool {
    critical(|| {
        let Some(st) = state() else { return false };
        let payload = &payload[..payload.len().min(128)];
        udp_emit(st, BROADCAST, CHAT_PORT, CHAT_PORT, payload)
    })
}

/// M26 relay (TCP). The chat app connects here to exchange the
/// HELLO/JOIN/PART/MSG protocol when an address is configured (fw_cfg
/// opt/veil.relay); absent one, chat stays in the M20 UDP-broadcast mode.
static mut RELAY: Option<([u8; 4], u16)> = None;

pub fn set_relay(addr: Option<([u8; 4], u16)>) {
    unsafe { *core::ptr::addr_of_mut!(RELAY) = addr };
}

pub fn relay_addr() -> Option<([u8; 4], u16)> {
    unsafe { *core::ptr::addr_of!(RELAY) }
}

/// Parse "a.b.c.d:port" (the opt/veil.relay fw_cfg string).
pub fn parse_relay(s: &str) -> Option<([u8; 4], u16)> {
    let (ip, port) = s.trim().split_once(':')?;
    let mut out = [0u8; 4];
    let mut it = ip.split('.');
    for b in out.iter_mut() {
        *b = it.next()?.parse().ok()?;
    }
    it.next().is_none().then_some(())?;
    Some((out, port.parse().ok()?))
}

/// M19b: client-side UDP send (DNS query, NTP request). `sport` must be
/// in 49152..61000 — replies queue in udp_rx for `udp_poll`. Returns
/// false on an ARP miss (a request went out; retry after a yield).
pub fn udp_send_to(dst: Ip, dport: u16, sport: u16, payload: &[u8]) -> bool {
    critical(|| {
        let Some(st) = state() else { return false };
        udp_emit(st, dst, dport, sport, payload)
    })
}

/// Pop one datagram received on client port `sport`.
pub fn udp_poll(sport: u16) -> Option<Vec<u8>> {
    critical(|| {
        let st = state()?;
        let idx = st.udp_rx.iter().position(|(p, _)| *p == sport)?;
        Some(st.udp_rx.remove(idx).1)
    })
}

/// The DNS resolver to use: slirp puts one at 10.0.2.3; otherwise assume
/// the gateway forwards DNS (typical home router).
pub fn dns_server() -> Option<Ip> {
    critical(|| {
        let st = state()?;
        Some(if st.gw == [10, 0, 2, 2] { [10, 0, 2, 3] } else { st.gw })
    })
}

// --- M19b: DNS resolver + NTP client (blocking, boot-time) -------------------

fn cnt_now() -> u64 {
    let c: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) c, options(nomem, nostack)) };
    c
}

fn cnt_freq() -> u64 {
    let f: u64;
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) f, options(nomem, nostack)) };
    f
}

/// Blocking UDP request/reply with retransmit. Sends `payload` to
/// `dst:dport` from ephemeral `sport`, then spins — IRQs stay unmasked, so
/// the reply arrives on the netdev IRQ and queues in `udp_rx` — until
/// `udp_poll` yields it or `timeout_ms` elapses, resending every
/// `retry_ms` to recover from the first-packet ARP miss. cntpct (not the
/// software tick, which is stopped this early in boot) drives the clock.
fn udp_request(
    dst: Ip,
    dport: u16,
    sport: u16,
    payload: &[u8],
    timeout_ms: u64,
    retry_ms: u64,
) -> Option<Vec<u8>> {
    let freq = cnt_freq();
    let start = cnt_now();
    let deadline = start + freq * timeout_ms / 1000;
    let mut next_send = start;
    loop {
        let now = cnt_now();
        if now >= next_send {
            udp_send_to(dst, dport, sport, payload);
            next_send = now + freq * retry_ms / 1000;
        }
        if let Some(reply) = udp_poll(sport) {
            return Some(reply);
        }
        if now >= deadline {
            return None;
        }
        core::hint::spin_loop();
    }
}

/// Build a DNS A-record query for `name`.
fn dns_build(name: &str, txid: u16) -> Vec<u8> {
    let mut q = Vec::new();
    q.extend_from_slice(&txid.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00]); // flags: recursion desired
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]); // qd=1, an/ns/ar=0
    for label in name.split('.') {
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0); // root label
    q.extend_from_slice(&[0, 1, 0, 1]); // QTYPE=A, QCLASS=IN
    q
}

/// Advance past a DNS name (labels or a compression pointer).
fn dns_skip_name(p: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *p.get(i)?;
        if len & 0xc0 == 0xc0 {
            return Some(i + 2); // pointer terminates the name
        }
        if len == 0 {
            return Some(i + 1);
        }
        i += 1 + len as usize;
    }
}

/// Pull the first A record (IPv4 address) out of a DNS response.
fn dns_parse(resp: &[u8], txid: u16) -> Option<Ip> {
    if resp.len() < 12 || resp[0..2] != txid.to_be_bytes() {
        return None;
    }
    let qd = u16::from_be_bytes([resp[4], resp[5]]);
    let an = u16::from_be_bytes([resp[6], resp[7]]);
    let mut i = 12;
    for _ in 0..qd {
        i = dns_skip_name(resp, i)? + 4; // skip qtype + qclass
    }
    for _ in 0..an {
        i = dns_skip_name(resp, i)?;
        let rtype = u16::from_be_bytes([*resp.get(i)?, *resp.get(i + 1)?]);
        let rdlen = u16::from_be_bytes([*resp.get(i + 8)?, *resp.get(i + 9)?]) as usize;
        i += 10;
        if rtype == 1 && rdlen == 4 {
            let a = resp.get(i..i + 4)?;
            return Some([a[0], a[1], a[2], a[3]]);
        }
        i += rdlen;
    }
    None
}

/// NTP transmit-timestamp seconds, converted from the 1900 epoch to unix.
fn ntp_parse(resp: &[u8]) -> Option<u64> {
    if resp.len() < 48 {
        return None;
    }
    const NTP_UNIX_DELTA: u64 = 2_208_988_800; // seconds 1900-01-01 -> 1970
    let secs = u32::from_be_bytes([resp[40], resp[41], resp[42], resp[43]]) as u64;
    (secs > NTP_UNIX_DELTA).then(|| secs - NTP_UNIX_DELTA)
}

/// M19b: resolve `host`, query it for the time, return real UTC seconds.
/// Blocking with a few-second budget; any timeout (no NIC config, no DNS,
/// unreachable server) returns None so the clock falls back to time-since-
/// boot. The single NTP exchange the spec asks for, plus the DNS lookup it
/// implies.
pub fn ntp_sync(host: &str) -> Option<u64> {
    let dns = dns_server()?;
    let txid = isn() as u16 | 1;
    kprintln!("DNS: resolving {host} via {}", fmt_ip(&dns));
    let query = dns_build(host, txid);
    let reply = udp_request(dns, 53, 50000, &query, 4000, 300)?;
    let server = dns_parse(&reply, txid)?;
    kprintln!("NTP: querying {} ({host}) :123", fmt_ip(&server));
    let mut req = alloc::vec![0u8; 48];
    req[0] = 0x1b; // LI=0, VN=3, Mode=3 (client)
    let reply = udp_request(server, 123, 50001, &req, 4000, 500)?;
    ntp_parse(&reply)
}

/// M20: pop one received chat datagram (the desktop loop polls this).
pub fn chat_take() -> Option<Vec<u8>> {
    critical(|| {
        let st = state()?;
        if st.chat_rx.is_empty() {
            None
        } else {
            Some(st.chat_rx.remove(0))
        }
    })
}

/// Active open. Returns immediately with the handle; the handshake (and,
/// for loopback, often the whole exchange) proceeds underneath. Callers
/// poll tcp_read — a failed connect surfaces as Eof.
pub fn tcp_connect(dst: Ip, port: u16) -> Option<Handle> {
    critical(|| {
        let st = state()?;
        let lport = st.next_port;
        st.next_port = if st.next_port >= 60000 { 49152 } else { st.next_port + 1 };
        let iss = isn();
        let conn = Conn {
            tag: st.next_gen,
            state: TcpState::SynSent,
            remote_ip: dst,
            remote_port: port,
            local_port: lport,
            iss,
            snd_una: iss,
            snd_nxt: iss.wrapping_add(1),
            rcv_nxt: 0,
            peer_win: 0,
            peer_mss: 536,
            sendq: Vec::new(),
            recvq: Vec::new(),
            fin_sent: false,
            fin_acked: false,
            app_closed: false,
            accepted: true, // the caller owns it from birth
            rto_left: RTO_TICKS,
            retries: 0,
            time_wait_left: 0,
            rx_bytes: 0,
            tx_bytes: 0,
        };
        let tag = conn.tag;
        st.next_gen += 1;
        let ci = match st.conns.iter().position(|c| c.is_none()) {
            Some(i) => {
                st.conns[i] = Some(conn);
                i
            }
            None => {
                st.conns.push(Some(conn));
                st.conns.len() - 1
            }
        };
        send_segment(st, ci, iss, FLAG_SYN, &[]);
        pump_loopback(st);
        Some(Handle { idx: ci, tag })
    })
}

fn lookup(st: &mut State, h: Handle) -> Option<&mut Conn> {
    st.conns.get_mut(h.idx)?.as_mut().filter(|c| c.tag == h.tag)
}

/// First not-yet-accepted connection past the handshake on `port`.
pub fn tcp_accept(port: u16) -> Option<Handle> {
    critical(|| {
        let st = state()?;
        for (idx, slot) in st.conns.iter_mut().enumerate() {
            if let Some(c) = slot {
                if !c.accepted
                    && c.local_port == port
                    && !matches!(c.state, TcpState::SynRcvd | TcpState::Closed)
                {
                    c.accepted = true;
                    return Some(Handle { idx, tag: c.tag });
                }
            }
        }
        None
    })
}

pub fn tcp_remote(h: Handle) -> Option<(Ip, u16)> {
    critical(|| {
        let st = state()?;
        lookup(st, h).map(|c| (c.remote_ip, c.remote_port))
    })
}

pub fn tcp_read(h: Handle, buf: &mut [u8]) -> TcpRead {
    critical(|| {
        let Some(st) = state() else { return TcpRead::Eof };
        let Some(c) = lookup(st, h) else { return TcpRead::Eof };
        if !c.recvq.is_empty() {
            let n = buf.len().min(c.recvq.len());
            buf[..n].copy_from_slice(&c.recvq[..n]);
            c.recvq.drain(..n);
            return TcpRead::Data(n);
        }
        match c.state {
            TcpState::Established
            | TcpState::SynSent
            | TcpState::SynRcvd
            | TcpState::FinWait1
            | TcpState::FinWait2 => TcpRead::Empty,
            _ => TcpRead::Eof, // peer FIN'd (CloseWait+) or conn died
        }
    })
}

/// Queue bytes for sending; returns how many fit. The caller loops + yields.
pub fn tcp_write(h: Handle, data: &[u8]) -> usize {
    critical(|| {
        let Some(st) = state() else { return data.len() };
        let n = {
            let Some(c) = lookup(st, h) else { return data.len() };
            if c.app_closed || matches!(c.state, TcpState::Closed | TcpState::TimeWait) {
                return data.len(); // sink it: connection is gone anyway
            }
            let n = data.len().min(SEND_CAP - c.sendq.len());
            c.sendq.extend_from_slice(&data[..n]);
            n
        };
        if n > 0 {
            tcp_output(st, h.idx);
            pump_loopback(st);
        }
        n
    })
}

/// Bytes queued but not yet acknowledged by the peer.
pub fn tcp_unacked(h: Handle) -> usize {
    critical(|| {
        let Some(st) = state() else { return 0 };
        lookup(st, h).map_or(0, |c| c.sendq.len())
    })
}

/// Orderly close: a FIN goes out once queued data drains.
pub fn tcp_close(h: Handle) {
    critical(|| {
        let Some(st) = state() else { return };
        let dead = {
            let Some(c) = lookup(st, h) else { return };
            if c.app_closed {
                return;
            }
            c.app_closed = true;
            c.state == TcpState::Closed
        };
        if dead {
            st.conns[h.idx] = None;
        } else {
            tcp_output(st, h.idx);
            pump_loopback(st);
        }
    })
}
