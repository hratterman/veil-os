//! M33: TLS 1.3 client (RFC 8446), cipher TLS_CHACHA20_POLY1305_SHA256, group
//! X25519. Pure from-scratch crypto (`crate::crypto`). Certificate chain
//! validation is intentionally skipped (this is a demo OS) — we verify only the
//! server Finished MAC, which proves the handshake transcript and keys match.

use crate::crypto::chacha20::{chacha20poly1305_decrypt, chacha20poly1305_encrypt};
use crate::crypto::hkdf::{derive_secret, expand_label, hkdf_extract};
use crate::crypto::sha256::{hmac_sha256, sha256};
use crate::crypto::x25519::{x25519, x25519_base};
use crate::{kprintln, net, scheduler, timer};
use alloc::vec;
use alloc::vec::Vec;

const REC_HANDSHAKE: u8 = 22;
const REC_APPDATA: u8 = 23;
const REC_CCS: u8 = 20;
const REC_ALERT: u8 = 21;

const HS_CLIENT_HELLO: u8 = 1;
const HS_SERVER_HELLO: u8 = 2;
const HS_ENCRYPTED_EXT: u8 = 8;
const HS_CERTIFICATE: u8 = 11;
const HS_CERT_VERIFY: u8 = 15;
const HS_FINISHED: u8 = 20;

// --- small writer helpers ----------------------------------------------------

fn u16b(v: usize) -> [u8; 2] {
    (v as u16).to_be_bytes()
}

/// Append `body` prefixed by a 2-byte length.
fn push_vec16(out: &mut Vec<u8>, body: &[u8]) {
    out.extend_from_slice(&u16b(body.len()));
    out.extend_from_slice(body);
}

fn extension(ext_type: u16, data: &[u8]) -> Vec<u8> {
    let mut e = Vec::with_capacity(4 + data.len());
    e.extend_from_slice(&ext_type.to_be_bytes());
    e.extend_from_slice(&u16b(data.len()));
    e.extend_from_slice(data);
    e
}

// --- record layer ------------------------------------------------------------

fn make_nonce(iv: &[u8; 12], seq: u64) -> [u8; 12] {
    let mut n = *iv;
    let s = seq.to_be_bytes();
    for i in 0..8 {
        n[4 + i] ^= s[i];
    }
    n
}

/// Encrypt `content` (inner type `ctype`) into a TLS 1.3 ciphertext record.
fn encrypt_record(key: &[u8; 32], iv: &[u8; 12], seq: u64, ctype: u8, content: &[u8]) -> Vec<u8> {
    let mut inner = content.to_vec();
    inner.push(ctype); // TLSInnerPlaintext: content || type (no padding)
    let len = inner.len() + 16; // + Poly1305 tag
    let header = [REC_APPDATA, 0x03, 0x03, (len >> 8) as u8, len as u8];
    let nonce = make_nonce(iv, seq);
    let ct = chacha20poly1305_encrypt(key, &nonce, &header, &inner);
    let mut rec = header.to_vec();
    rec.extend_from_slice(&ct);
    rec
}

/// Decrypt one ciphertext record body; returns (inner content type, content).
fn decrypt_record(
    key: &[u8; 32], iv: &[u8; 12], seq: u64, header: &[u8], body: &[u8],
) -> Option<(u8, Vec<u8>)> {
    let nonce = make_nonce(iv, seq);
    let mut plain = chacha20poly1305_decrypt(key, &nonce, header, body)?;
    // Strip zero padding; the last non-zero byte is the real content type.
    while let Some(&0) = plain.last() {
        plain.pop();
    }
    let ctype = plain.pop()?;
    Some((ctype, plain))
}

/// Buffers the TCP byte stream and hands back whole TLS records.
struct Rx {
    sock: net::Handle,
    buf: Vec<u8>,
}

impl Rx {
    /// Next record as (record_type, 5-byte header, body), or None on
    /// EOF/timeout. `deadline` is an absolute `timer::ticks()` value.
    fn record(&mut self, deadline: u64) -> Option<(u8, Vec<u8>, Vec<u8>)> {
        loop {
            if self.buf.len() >= 5 {
                let len = ((self.buf[3] as usize) << 8) | self.buf[4] as usize;
                if self.buf.len() >= 5 + len {
                    let header = self.buf[..5].to_vec();
                    let body = self.buf[5..5 + len].to_vec();
                    self.buf.drain(..5 + len);
                    return Some((header[0], header, body));
                }
            }
            let mut tmp = [0u8; 2048];
            match net::tcp_read(self.sock, &mut tmp) {
                net::TcpRead::Data(n) => self.buf.extend_from_slice(&tmp[..n]),
                net::TcpRead::Empty => {
                    if timer::ticks() > deadline {
                        return None;
                    }
                    scheduler::yield_now();
                }
                net::TcpRead::Eof => return None,
            }
        }
    }
}

// --- key schedule ------------------------------------------------------------

struct Keys {
    key: [u8; 32],
    iv: [u8; 12],
    seq: u64,
}

fn traffic_keys(secret: &[u8; 32]) -> Keys {
    let k = expand_label(secret, "key", &[], 32);
    let v = expand_label(secret, "iv", &[], 12);
    let mut key = [0u8; 32];
    let mut iv = [0u8; 12];
    key.copy_from_slice(&k);
    iv.copy_from_slice(&v);
    Keys { key, iv, seq: 0 }
}

// --- connection --------------------------------------------------------------

pub struct TlsConn {
    rx: Rx,
    client: Keys, // application traffic
    server: Keys,
}

impl TlsConn {
    /// Encrypt and send `data` as one or more application_data records.
    pub fn write(&mut self, data: &[u8]) {
        for chunk in data.chunks(16384) {
            let rec = encrypt_record(&self.client.key, &self.client.iv, self.client.seq, REC_APPDATA, chunk);
            self.client.seq += 1;
            net::tcp_write(self.rx.sock, &rec);
        }
    }

    /// Read the next chunk of decrypted application data, skipping post-
    /// handshake messages (NewSessionTicket etc.). None on close/timeout.
    pub fn read(&mut self, deadline: u64) -> Option<Vec<u8>> {
        loop {
            let (rtype, header, body) = self.rx.record(deadline)?;
            match rtype {
                REC_CCS => continue, // ignore late change_cipher_spec
                REC_APPDATA => {
                    let (ctype, content) =
                        decrypt_record(&self.server.key, &self.server.iv, self.server.seq, &header, &body)?;
                    self.server.seq += 1;
                    match ctype {
                        REC_APPDATA => return Some(content),
                        REC_HANDSHAKE => continue, // NewSessionTicket — ignore
                        REC_ALERT => return None,  // close_notify etc.
                        _ => continue,
                    }
                }
                _ => continue,
            }
        }
    }

    pub fn close(&mut self) {
        net::tcp_close(self.rx.sock);
    }
}

// --- handshake ---------------------------------------------------------------

fn build_client_hello(pubkey: &[u8; 32], host: &str, random: &[u8; 32], session_id: &[u8; 32]) -> Vec<u8> {
    let mut exts = Vec::new();
    // server_name (SNI)
    {
        let mut sni = Vec::new();
        let mut list = Vec::new();
        list.push(0); // name_type host_name
        list.extend_from_slice(&u16b(host.len()));
        list.extend_from_slice(host.as_bytes());
        push_vec16(&mut sni, &list);
        exts.extend_from_slice(&extension(0x0000, &sni));
    }
    // supported_groups: x25519
    exts.extend_from_slice(&extension(0x000a, &[0x00, 0x02, 0x00, 0x1d]));
    // supported_versions: TLS 1.3
    exts.extend_from_slice(&extension(0x002b, &[0x02, 0x03, 0x04]));
    // signature_algorithms
    exts.extend_from_slice(&extension(
        0x000d,
        &[0x00, 0x06, 0x04, 0x03, 0x08, 0x04, 0x04, 0x01],
    ));
    // key_share: x25519 public
    {
        let mut entry = Vec::new();
        entry.extend_from_slice(&[0x00, 0x1d]);
        entry.extend_from_slice(&u16b(32));
        entry.extend_from_slice(pubkey);
        let mut ks = Vec::new();
        push_vec16(&mut ks, &entry);
        exts.extend_from_slice(&extension(0x0033, &ks));
    }

    let mut body = Vec::new();
    body.extend_from_slice(&[0x03, 0x03]); // legacy_version TLS 1.2
    body.extend_from_slice(random);
    body.push(32); // legacy_session_id length
    body.extend_from_slice(session_id);
    body.extend_from_slice(&[0x00, 0x02, 0x13, 0x03]); // cipher_suites: chacha20-poly1305
    body.extend_from_slice(&[0x01, 0x00]); // compression: null
    body.extend_from_slice(&u16b(exts.len()));
    body.extend_from_slice(&exts);

    // Wrap in handshake header (type=1, u24 length).
    let mut msg = Vec::with_capacity(4 + body.len());
    msg.push(HS_CLIENT_HELLO);
    msg.push((body.len() >> 16) as u8);
    msg.push((body.len() >> 8) as u8);
    msg.push(body.len() as u8);
    msg.extend_from_slice(&body);
    msg
}

/// Extract the server's x25519 key_share from a ServerHello body (the bytes
/// after the 4-byte handshake header). Returns None on HRR / wrong params.
fn parse_server_hello(sh: &[u8]) -> Option<[u8; 32]> {
    let mut i = 0;
    i += 2; // legacy_version
    // HelloRetryRequest sentinel random — we don't support HRR.
    const HRR: [u8; 32] = [
        0xCF, 0x21, 0xAD, 0x74, 0xE5, 0x9A, 0x61, 0x11, 0xBE, 0x1D, 0x8C, 0x02, 0x1E, 0x65, 0xB8, 0x91,
        0xC2, 0xA2, 0x11, 0x16, 0x7A, 0xBB, 0x8C, 0x5E, 0x07, 0x9E, 0x09, 0xE2, 0xC8, 0xA8, 0x33, 0x9C,
    ];
    if sh.get(i..i + 32)? == HRR {
        kprintln!("TLS: HelloRetryRequest unsupported");
        return None;
    }
    i += 32; // random
    let sid_len = *sh.get(i)? as usize;
    i += 1 + sid_len; // session_id echo
    let cipher = [*sh.get(i)?, *sh.get(i + 1)?];
    if cipher != [0x13, 0x03] {
        kprintln!("TLS: server chose cipher {:02x}{:02x}, want 1303", cipher[0], cipher[1]);
        return None;
    }
    i += 2; // cipher_suite
    i += 1; // legacy_compression
    let ext_len = ((*sh.get(i)? as usize) << 8) | *sh.get(i + 1)? as usize;
    i += 2;
    let ext_end = i + ext_len;
    while i + 4 <= ext_end {
        let etype = ((sh[i] as u16) << 8) | sh[i + 1] as u16;
        let elen = ((sh[i + 2] as usize) << 8) | sh[i + 3] as usize;
        let edata = sh.get(i + 4..i + 4 + elen)?;
        if etype == 0x0033 {
            // key_share: group(2) | key_len(2) | key
            if edata.len() >= 4 && edata[0..2] == [0x00, 0x1d] {
                let klen = ((edata[2] as usize) << 8) | edata[3] as usize;
                let key = edata.get(4..4 + klen)?;
                if key.len() == 32 {
                    let mut out = [0u8; 32];
                    out.copy_from_slice(key);
                    return Some(out);
                }
            }
        }
        i += 4 + elen;
    }
    kprintln!("TLS: no x25519 key_share in ServerHello");
    None
}

/// Full TLS 1.3 handshake to `host:port`. Returns a ready TlsConn on success.
pub fn tls_connect(host: &str, port: u16) -> Option<TlsConn> {
    let ip = net::dns_resolve(host)?;
    kprintln!("TLS: connecting {host} ({}) :{port}", net::fmt_ip(&ip));
    let sock = net::tcp_connect(ip, port)?;

    // Our ephemeral X25519 keypair + ClientHello randomness.
    let mut priv_key = [0u8; 32];
    let mut random = [0u8; 32];
    let mut session_id = [0u8; 32];
    net::rng_fill(&mut priv_key);
    net::rng_fill(&mut random);
    net::rng_fill(&mut session_id);
    let pub_key = x25519_base(&priv_key);

    let ch = build_client_hello(&pub_key, host, &random, &session_id);
    let mut transcript = ch.clone();
    // ClientHello as a plaintext handshake record.
    {
        let mut rec = vec![REC_HANDSHAKE, 0x03, 0x01];
        rec.extend_from_slice(&u16b(ch.len()));
        rec.extend_from_slice(&ch);
        net::tcp_write(sock, &rec);
    }

    let mut rx = Rx { sock, buf: Vec::new() };
    let deadline = timer::ticks() + 600; // ~12 s at 50 Hz

    // ServerHello (plaintext handshake record).
    let server_pub = loop {
        let (rtype, _h, body) = rx.record(deadline)?;
        if rtype == REC_CCS {
            continue;
        }
        if rtype != REC_HANDSHAKE || body.first() != Some(&HS_SERVER_HELLO) {
            kprintln!("TLS: expected ServerHello, got record type {rtype}");
            net::tcp_close(sock);
            return None;
        }
        transcript.extend_from_slice(&body);
        let sp = parse_server_hello(&body[4..]).or_else(|| {
            net::tcp_close(sock);
            None
        })?;
        break sp;
    };
    kprintln!("TLS: ServerHello OK, deriving handshake keys");

    // Key schedule through the handshake secret.
    let shared = x25519(&priv_key, &server_pub);
    let empty_hash = sha256(b"");
    let early = hkdf_extract(&[], &[0u8; 32]);
    let derived_es = derive_secret(&early, "derived", &empty_hash);
    let handshake_secret = hkdf_extract(&derived_es, &shared);
    let th_chsh = sha256(&transcript); // Hash(ClientHello..ServerHello)
    let c_hs = derive_secret(&handshake_secret, "c hs traffic", &th_chsh);
    let s_hs = derive_secret(&handshake_secret, "s hs traffic", &th_chsh);
    let mut server_hs = traffic_keys(&s_hs);

    // Read + decrypt the encrypted handshake flight, parsing messages out of a
    // running buffer (a message may span records). Verify the server Finished.
    let mut hs_data: Vec<u8> = Vec::new();
    let mut server_finished_ok = false;
    'flight: loop {
        // Consume any complete handshake messages already buffered.
        while hs_data.len() >= 4 {
            let mlen = ((hs_data[1] as usize) << 16) | ((hs_data[2] as usize) << 8) | hs_data[3] as usize;
            if hs_data.len() < 4 + mlen {
                break;
            }
            let mtype = hs_data[0];
            let msg: Vec<u8> = hs_data.drain(..4 + mlen).collect();
            if mtype == HS_FINISHED {
                // verify_data = HMAC(finished_key, Hash(transcript so far)).
                let fk = expand_label(&s_hs, "finished", &[], 32);
                let expected = hmac_sha256(&fk, &sha256(&transcript));
                if msg.len() == 4 + 32 && expected == msg[4..] {
                    server_finished_ok = true;
                } else {
                    kprintln!("TLS: server Finished MAC mismatch");
                }
                transcript.extend_from_slice(&msg);
                break 'flight;
            }
            // EncryptedExtensions / Certificate / CertificateVerify: cert
            // validation deliberately skipped — just keep the transcript.
            let _ = (HS_ENCRYPTED_EXT, HS_CERTIFICATE, HS_CERT_VERIFY);
            transcript.extend_from_slice(&msg);
        }
        // Need more record data.
        let (rtype, header, body) = rx.record(deadline)?;
        match rtype {
            REC_CCS => {}
            REC_APPDATA => {
                let (ctype, content) =
                    decrypt_record(&server_hs.key, &server_hs.iv, server_hs.seq, &header, &body)?;
                server_hs.seq += 1;
                if ctype == REC_HANDSHAKE {
                    hs_data.extend_from_slice(&content);
                } else if ctype == REC_ALERT {
                    kprintln!("TLS: alert during handshake");
                    net::tcp_close(sock);
                    return None;
                }
            }
            _ => {}
        }
    }

    if !server_finished_ok {
        net::tcp_close(sock);
        return None;
    }
    kprintln!("TLS: server Finished verified");

    // Application traffic secrets use the transcript through server Finished.
    let derived_hs = derive_secret(&handshake_secret, "derived", &empty_hash);
    let master = hkdf_extract(&derived_hs, &[0u8; 32]);
    let th_sf = sha256(&transcript);
    let c_ap = derive_secret(&master, "c ap traffic", &th_sf);
    let s_ap = derive_secret(&master, "s ap traffic", &th_sf);

    // Client Finished (encrypted with the client handshake key).
    let cfk = expand_label(&c_hs, "finished", &[], 32);
    let cverify = hmac_sha256(&cfk, &sha256(&transcript));
    let mut fin_msg = vec![HS_FINISHED, 0, 0, 32];
    fin_msg.extend_from_slice(&cverify);
    // change_cipher_spec for middlebox compatibility, then the encrypted Finished.
    net::tcp_write(sock, &[REC_CCS, 0x03, 0x03, 0x00, 0x01, 0x01]);
    let mut client_hs = traffic_keys(&c_hs);
    let fin_rec = encrypt_record(&client_hs.key, &client_hs.iv, client_hs.seq, REC_HANDSHAKE, &fin_msg);
    client_hs.seq += 1;
    net::tcp_write(sock, &fin_rec);

    kprintln!("TLS: handshake complete — application keys ready");
    Some(TlsConn {
        rx,
        client: traffic_keys(&c_ap),
        server: traffic_keys(&s_ap),
    })
}
