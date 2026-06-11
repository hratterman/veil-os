//! ChaCha20 + Poly1305 AEAD (RFC 8439). Pure integer math, no_std.
//! Poly1305 follows the public-domain poly1305-donna 32-bit limb structure.

use alloc::vec::Vec;

#[inline]
fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(16);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(12);
    s[a] = s[a].wrapping_add(s[b]); s[d] ^= s[a]; s[d] = s[d].rotate_left(8);
    s[c] = s[c].wrapping_add(s[d]); s[b] ^= s[c]; s[b] = s[b].rotate_left(7);
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

pub fn chacha20_block(key: &[u8; 32], counter: u32, nonce: &[u8; 12]) -> [u8; 64] {
    let mut state = [0u32; 16];
    state[0] = 0x6170_7865;
    state[1] = 0x3320_646e;
    state[2] = 0x7962_2d32;
    state[3] = 0x6b20_6574;
    for i in 0..8 {
        state[4 + i] = le32(&key[i * 4..]);
    }
    state[12] = counter;
    for i in 0..3 {
        state[13 + i] = le32(&nonce[i * 4..]);
    }
    let mut w = state;
    for _ in 0..10 {
        qr(&mut w, 0, 4, 8, 12);
        qr(&mut w, 1, 5, 9, 13);
        qr(&mut w, 2, 6, 10, 14);
        qr(&mut w, 3, 7, 11, 15);
        qr(&mut w, 0, 5, 10, 15);
        qr(&mut w, 1, 6, 11, 12);
        qr(&mut w, 2, 7, 8, 13);
        qr(&mut w, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&w[i].wrapping_add(state[i]).to_le_bytes());
    }
    out
}

/// XOR `data` with the ChaCha20 keystream starting at block `counter0`.
pub fn chacha20_xor(key: &[u8; 32], counter0: u32, nonce: &[u8; 12], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut counter = counter0;
    let mut i = 0;
    while i < data.len() {
        let ks = chacha20_block(key, counter, nonce);
        let n = core::cmp::min(64, data.len() - i);
        for j in 0..n {
            out.push(data[i + j] ^ ks[j]);
        }
        i += n;
        counter = counter.wrapping_add(1);
    }
    out
}

pub fn poly1305_mac(key: &[u8; 32], msg: &[u8]) -> [u8; 16] {
    // Clamp r.
    let t0 = le32(&key[0..]);
    let t1 = le32(&key[4..]);
    let t2 = le32(&key[8..]);
    let t3 = le32(&key[12..]);
    let r0 = t0 & 0x3ff_ffff;
    let r1 = ((t0 >> 26) | (t1 << 6)) & 0x3ff_ff03;
    let r2 = ((t1 >> 20) | (t2 << 12)) & 0x3ff_c0ff;
    let r3 = ((t2 >> 14) | (t3 << 18)) & 0x3f0_3fff;
    let r4 = (t3 >> 8) & 0x00f_ffff;
    let s1 = r1.wrapping_mul(5);
    let s2 = r2.wrapping_mul(5);
    let s3 = r3.wrapping_mul(5);
    let s4 = r4.wrapping_mul(5);

    let (mut h0, mut h1, mut h2, mut h3, mut h4) = (0u32, 0u32, 0u32, 0u32, 0u32);

    let mut i = 0;
    while i < msg.len() {
        let n = core::cmp::min(16, msg.len() - i);
        let mut block = [0u8; 16];
        block[..n].copy_from_slice(&msg[i..i + n]);
        // The appended 1 bit: a full block adds 2^128 (hibit), a partial block
        // puts the 1 byte right after the data.
        let hibit: u32 = if n == 16 {
            1 << 24
        } else {
            block[n] = 1;
            0
        };
        let b0 = le32(&block[0..]);
        let b1 = le32(&block[4..]);
        let b2 = le32(&block[8..]);
        let b3 = le32(&block[12..]);
        h0 += b0 & 0x3ff_ffff;
        h1 += ((b0 >> 26) | (b1 << 6)) & 0x3ff_ffff;
        h2 += ((b1 >> 20) | (b2 << 12)) & 0x3ff_ffff;
        h3 += ((b2 >> 14) | (b3 << 18)) & 0x3ff_ffff;
        h4 += (b3 >> 8) | hibit;

        let d0 = h0 as u64 * r0 as u64 + h1 as u64 * s4 as u64 + h2 as u64 * s3 as u64
            + h3 as u64 * s2 as u64 + h4 as u64 * s1 as u64;
        let d1 = h0 as u64 * r1 as u64 + h1 as u64 * r0 as u64 + h2 as u64 * s4 as u64
            + h3 as u64 * s3 as u64 + h4 as u64 * s2 as u64;
        let d2 = h0 as u64 * r2 as u64 + h1 as u64 * r1 as u64 + h2 as u64 * r0 as u64
            + h3 as u64 * s4 as u64 + h4 as u64 * s3 as u64;
        let d3 = h0 as u64 * r3 as u64 + h1 as u64 * r2 as u64 + h2 as u64 * r1 as u64
            + h3 as u64 * r0 as u64 + h4 as u64 * s4 as u64;
        let d4 = h0 as u64 * r4 as u64 + h1 as u64 * r3 as u64 + h2 as u64 * r2 as u64
            + h3 as u64 * r1 as u64 + h4 as u64 * r0 as u64;

        let mut c = (d0 >> 26) as u32;
        h0 = d0 as u32 & 0x3ff_ffff;
        let d1 = d1 + c as u64;
        c = (d1 >> 26) as u32;
        h1 = d1 as u32 & 0x3ff_ffff;
        let d2 = d2 + c as u64;
        c = (d2 >> 26) as u32;
        h2 = d2 as u32 & 0x3ff_ffff;
        let d3 = d3 + c as u64;
        c = (d3 >> 26) as u32;
        h3 = d3 as u32 & 0x3ff_ffff;
        let d4 = d4 + c as u64;
        c = (d4 >> 26) as u32;
        h4 = d4 as u32 & 0x3ff_ffff;
        h0 += c * 5;
        c = h0 >> 26;
        h0 &= 0x3ff_ffff;
        h1 += c;

        i += n;
    }

    // Fully carry h.
    let mut c = h1 >> 26; h1 &= 0x3ff_ffff; h2 += c;
    c = h2 >> 26; h2 &= 0x3ff_ffff; h3 += c;
    c = h3 >> 26; h3 &= 0x3ff_ffff; h4 += c;
    c = h4 >> 26; h4 &= 0x3ff_ffff; h0 += c * 5;
    c = h0 >> 26; h0 &= 0x3ff_ffff; h1 += c;

    // Compute h - p.
    let mut g0 = h0.wrapping_add(5); c = g0 >> 26; g0 &= 0x3ff_ffff;
    let mut g1 = h1.wrapping_add(c); c = g1 >> 26; g1 &= 0x3ff_ffff;
    let mut g2 = h2.wrapping_add(c); c = g2 >> 26; g2 &= 0x3ff_ffff;
    let mut g3 = h3.wrapping_add(c); c = g3 >> 26; g3 &= 0x3ff_ffff;
    let mut g4 = h4.wrapping_add(c).wrapping_sub(1 << 26);

    // Select h if h < p, else g (constant-time mask via the borrow sign bit).
    let mask = (g4 >> 31).wrapping_sub(1);
    g0 &= mask; g1 &= mask; g2 &= mask; g3 &= mask; g4 &= mask;
    let nmask = !mask;
    h0 = (h0 & nmask) | g0;
    h1 = (h1 & nmask) | g1;
    h2 = (h2 & nmask) | g2;
    h3 = (h3 & nmask) | g3;
    h4 = (h4 & nmask) | g4;

    // Collapse the 26-bit limbs to four 32-bit words.
    let w0 = (h0 | (h1 << 26)) as u64;
    let w1 = ((h1 >> 6) | (h2 << 20)) as u64;
    let w2 = ((h2 >> 12) | (h3 << 14)) as u64;
    let w3 = ((h3 >> 18) | (h4 << 8)) as u64;

    // mac = (h + s) mod 2^128.
    let s0 = le32(&key[16..]) as u64;
    let s1k = le32(&key[20..]) as u64;
    let s2k = le32(&key[24..]) as u64;
    let s3k = le32(&key[28..]) as u64;
    let mut f = w0 + s0;
    let o0 = f as u32;
    f = w1 + s1k + (f >> 32);
    let o1 = f as u32;
    f = w2 + s2k + (f >> 32);
    let o2 = f as u32;
    f = w3 + s3k + (f >> 32);
    let o3 = f as u32;

    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&o0.to_le_bytes());
    out[4..8].copy_from_slice(&o1.to_le_bytes());
    out[8..12].copy_from_slice(&o2.to_le_bytes());
    out[12..16].copy_from_slice(&o3.to_le_bytes());
    out
}

fn poly_key(key: &[u8; 32], nonce: &[u8; 12]) -> [u8; 32] {
    let block = chacha20_block(key, 0, nonce);
    let mut k = [0u8; 32];
    k.copy_from_slice(&block[..32]);
    k
}

fn pad16(v: &mut Vec<u8>, len: usize) {
    let r = len % 16;
    if r != 0 {
        for _ in 0..(16 - r) {
            v.push(0);
        }
    }
}

/// RFC 8439 §2.8 AEAD construction: returns ciphertext || 16-byte tag.
pub fn chacha20poly1305_encrypt(
    key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8],
) -> Vec<u8> {
    let otk = poly_key(key, nonce);
    let ciphertext = chacha20_xor(key, 1, nonce, plaintext);
    let mut mac_data = Vec::new();
    mac_data.extend_from_slice(aad);
    pad16(&mut mac_data, aad.len());
    mac_data.extend_from_slice(&ciphertext);
    pad16(&mut mac_data, ciphertext.len());
    mac_data.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    mac_data.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    let tag = poly1305_mac(&otk, &mac_data);
    let mut out = ciphertext;
    out.extend_from_slice(&tag);
    out
}

/// Decrypts `ciphertext || tag`, verifying the tag (constant-time). Returns the
/// plaintext, or None on auth failure / malformed input.
pub fn chacha20poly1305_decrypt(
    key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], ct_and_tag: &[u8],
) -> Option<Vec<u8>> {
    if ct_and_tag.len() < 16 {
        return None;
    }
    let (ciphertext, tag) = ct_and_tag.split_at(ct_and_tag.len() - 16);
    let otk = poly_key(key, nonce);
    let mut mac_data = Vec::new();
    mac_data.extend_from_slice(aad);
    pad16(&mut mac_data, aad.len());
    mac_data.extend_from_slice(ciphertext);
    pad16(&mut mac_data, ciphertext.len());
    mac_data.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    mac_data.extend_from_slice(&(ciphertext.len() as u64).to_le_bytes());
    let expected = poly1305_mac(&otk, &mac_data);
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= expected[i] ^ tag[i];
    }
    if diff != 0 {
        return None;
    }
    Some(chacha20_xor(key, 1, nonce, ciphertext))
}
