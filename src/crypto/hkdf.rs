//! HKDF (RFC 5869) over HMAC-SHA256, plus the TLS 1.3 HKDF-Expand-Label /
//! Derive-Secret helpers (RFC 8446 §7.1). no_std.

use super::sha256::hmac_sha256;
use alloc::vec::Vec;

pub fn hkdf_extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    hmac_sha256(salt, ikm)
}

pub fn hkdf_expand(prk: &[u8], info: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len);
    let mut t: [u8; 32] = [0; 32];
    let mut tlen = 0usize;
    let mut counter = 1u8;
    while out.len() < len {
        // T(n) = HMAC(prk, T(n-1) | info | counter)
        let mut msg = Vec::with_capacity(tlen + info.len() + 1);
        msg.extend_from_slice(&t[..tlen]);
        msg.extend_from_slice(info);
        msg.push(counter);
        t = hmac_sha256(prk, &msg);
        tlen = 32;
        let take = core::cmp::min(32, len - out.len());
        out.extend_from_slice(&t[..take]);
        counter = counter.wrapping_add(1);
    }
    out
}

/// TLS 1.3 HKDF-Expand-Label (RFC 8446 §7.1).
pub fn expand_label(secret: &[u8], label: &str, context: &[u8], len: usize) -> Vec<u8> {
    // struct { uint16 length; opaque label<7..255>; opaque context<0..255>; }
    let full_label = alloc::format!("tls13 {label}");
    let mut info = Vec::with_capacity(2 + 1 + full_label.len() + 1 + context.len());
    info.extend_from_slice(&(len as u16).to_be_bytes());
    info.push(full_label.len() as u8);
    info.extend_from_slice(full_label.as_bytes());
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    hkdf_expand(secret, &info, len)
}

/// Derive-Secret(secret, label, messages) = Expand-Label(secret, label,
/// Hash(messages), Hash.length).
pub fn derive_secret(secret: &[u8], label: &str, transcript_hash: &[u8]) -> [u8; 32] {
    let v = expand_label(secret, label, transcript_hash, 32);
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    out
}
