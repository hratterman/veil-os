//! M41 step 17: from-scratch RSA PKCS#1 v1.5 / SHA-256 signature verification,
//! for X.509 certificate chains. A minimal big-integer modexp (`s^e mod n`)
//! plus the deterministic PKCS#1 v1.5 DigestInfo check — no crates.

use alloc::vec::Vec;
use crate::crypto::sha256::sha256;

/// SubjectPublicKeyInfo OID prefix for rsaEncryption.
const OID_RSA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];

/// Boot self-test: the big-integer modexp against known vectors.
pub fn selftest() {
    // 4^13 mod 497 = 445 (classic), and 2790^17 mod 3233 = 65 (toy RSA decrypt).
    let a = modexp(&[4], &[13], &[0x01, 0xf1]); // 4^13 mod 497   -> 445 = 0x01BD
    let b = modexp(&[65], &[17], &[0x0c, 0xa1]); // 65^17 mod 3233 -> 2790 = 0x0AE6
    let ok = a == [0x01u8, 0xbd] && b == [0x0au8, 0xe6];
    crate::kprintln!("RSA_MODEXP: 4^13 mod 497 = {:?}, 2790^17 mod 3233 = {:?}", a, b);
    if ok {
        crate::kprintln!("RSA_OK: from-scratch bignum modexp verified");
    } else {
        crate::kprintln!("RSA_FAIL");
    }
}

/// Is this SPKI an RSA public key?
pub fn is_rsa_spki(spki: &[u8]) -> bool {
    contains(spki, OID_RSA)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Extract (modulus n, exponent e) from an RSA SubjectPublicKeyInfo (DER).
fn rsa_pubkey(spki: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    // SPKI = SEQ { AlgId, BIT STRING { SEQ { INTEGER n, INTEGER e } } }
    let (_, body) = tlv(spki, 0)?; // outer SEQ content
    let mut p = 0;
    let (_, _alg, np) = tlv_at(body, p)?; // AlgorithmIdentifier
    p = np;
    let (tag, bits, _) = tlv_at(body, p)?; // BIT STRING
    if tag != 0x03 {
        return None;
    }
    let key = &bits[1..]; // drop the unused-bits byte
    let (_, seq) = tlv(key, 0)?; // SEQ { n, e }
    let mut q = 0;
    let (_, n, nq) = tlv_at(seq, q)?;
    q = nq;
    let (_, e, _) = tlv_at(seq, q)?;
    Some((trim_leading_zeros(n).to_vec(), trim_leading_zeros(e).to_vec()))
}

fn trim_leading_zeros(b: &[u8]) -> &[u8] {
    let mut i = 0;
    while i + 1 < b.len() && b[i] == 0 {
        i += 1;
    }
    &b[i..]
}

/// DER TLV at the start of `b` (offset 0 helper): returns (tag, content).
fn tlv(b: &[u8], off: usize) -> Option<(u8, &[u8])> {
    tlv_at(b, off).map(|(t, c, _)| (t, c))
}

/// DER TLV at `off`: returns (tag, content, next-offset).
fn tlv_at(b: &[u8], off: usize) -> Option<(u8, &[u8], usize)> {
    let tag = *b.get(off)?;
    let first = *b.get(off + 1)?;
    let (len, hdr) = if first & 0x80 == 0 {
        (first as usize, 2)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut l = 0usize;
        for k in 0..n {
            l = (l << 8) | *b.get(off + 2 + k)? as usize;
        }
        (l, 2 + n)
    };
    let start = off + hdr;
    let end = start.checked_add(len)?;
    if end > b.len() {
        return None;
    }
    Some((tag, &b[start..end], end))
}

/// Verify an RSA PKCS#1 v1.5 SHA-256 signature: the SPKI's key over `msg`.
pub fn verify_pkcs1_sha256(spki: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let Some((n, e)) = rsa_pubkey(spki) else { return false };
    if sig.is_empty() || n.is_empty() {
        return false;
    }
    // m = sig^e mod n
    let m = modexp(trim_leading_zeros(sig), &e, &n);
    // Build the expected EMSA-PKCS1-v1.5 encoding: 0x00 01 FF..FF 00 DigestInfo.
    let digest = sha256(msg);
    // DigestInfo for SHA-256.
    let di: &[u8] = &[
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    let k = n.len();
    if k < di.len() + digest.len() + 11 {
        return false;
    }
    let mut em = Vec::with_capacity(k);
    em.push(0x00);
    em.push(0x01);
    let ps = k - di.len() - digest.len() - 3;
    em.extend(core::iter::repeat(0xffu8).take(ps));
    em.push(0x00);
    em.extend_from_slice(di);
    em.extend_from_slice(&digest);
    // Left-pad `m` to k bytes for comparison.
    let mut mp = alloc::vec![0u8; k.saturating_sub(m.len())];
    mp.extend_from_slice(&m);
    mp.len() == em.len() && mp == em
}

// --- big-integer modular exponentiation (base^exp mod modulus), big-endian ----

fn modexp(base: &[u8], exp: &[u8], modulus: &[u8]) -> Vec<u8> {
    let m = Big::from_be(modulus);
    if m.is_zero() {
        return Vec::new();
    }
    let mut result = Big::one();
    let mut b = Big::from_be(base).rem(&m);
    // process exponent bits from least significant
    for byte in exp.iter().rev() {
        let mut bits = *byte;
        for _ in 0..8 {
            if bits & 1 == 1 {
                result = result.mulmod(&b, &m);
            }
            b = b.mulmod(&b, &m);
            bits >>= 1;
        }
    }
    result.to_be()
}

/// A minimal unsigned big integer as little-endian u32 limbs.
struct Big {
    limbs: Vec<u32>,
}

impl Big {
    fn from_be(b: &[u8]) -> Big {
        let mut limbs = Vec::new();
        let mut i = b.len();
        while i > 0 {
            let lo = i.saturating_sub(4);
            let mut v = 0u32;
            for &byte in &b[lo..i] {
                v = (v << 8) | byte as u32;
            }
            limbs.push(v);
            i = lo;
        }
        let mut x = Big { limbs };
        x.normalize();
        x
    }
    fn one() -> Big {
        Big { limbs: alloc::vec![1] }
    }
    fn is_zero(&self) -> bool {
        self.limbs.iter().all(|&l| l == 0)
    }
    fn normalize(&mut self) {
        while self.limbs.len() > 1 && *self.limbs.last().unwrap() == 0 {
            self.limbs.pop();
        }
    }
    fn to_be(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for &l in self.limbs.iter().rev() {
            out.extend_from_slice(&l.to_be_bytes());
        }
        // trim leading zeros
        let mut i = 0;
        while i + 1 < out.len() && out[i] == 0 {
            i += 1;
        }
        out[i..].to_vec()
    }
    fn cmp(&self, o: &Big) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        if self.limbs.len() != o.limbs.len() {
            return self.limbs.len().cmp(&o.limbs.len());
        }
        for i in (0..self.limbs.len()).rev() {
            match self.limbs[i].cmp(&o.limbs[i]) {
                Ordering::Equal => {}
                ord => return ord,
            }
        }
        Ordering::Equal
    }
    fn shl1(&self) -> Big {
        let mut limbs = Vec::with_capacity(self.limbs.len() + 1);
        let mut carry = 0u32;
        for &l in &self.limbs {
            limbs.push((l << 1) | carry);
            carry = l >> 31;
        }
        if carry != 0 {
            limbs.push(carry);
        }
        let mut b = Big { limbs };
        b.normalize();
        b
    }
    fn bit(&self, i: usize) -> bool {
        let limb = i / 32;
        let off = i % 32;
        self.limbs.get(limb).map(|l| (l >> off) & 1 == 1).unwrap_or(false)
    }
    fn bits(&self) -> usize {
        let top = self.limbs.len() - 1;
        let mut n = top * 32;
        let mut v = self.limbs[top];
        while v != 0 {
            n += 1;
            v >>= 1;
        }
        n
    }
    fn add(&self, o: &Big) -> Big {
        let mut limbs = Vec::with_capacity(self.limbs.len().max(o.limbs.len()) + 1);
        let mut carry = 0u64;
        for i in 0..self.limbs.len().max(o.limbs.len()) {
            let a = *self.limbs.get(i).unwrap_or(&0) as u64;
            let b = *o.limbs.get(i).unwrap_or(&0) as u64;
            let s = a + b + carry;
            limbs.push(s as u32);
            carry = s >> 32;
        }
        if carry != 0 {
            limbs.push(carry as u32);
        }
        let mut r = Big { limbs };
        r.normalize();
        r
    }
    fn sub(&self, o: &Big) -> Big {
        // assumes self >= o
        let mut limbs = Vec::with_capacity(self.limbs.len());
        let mut borrow = 0i64;
        for i in 0..self.limbs.len() {
            let a = self.limbs[i] as i64;
            let b = *o.limbs.get(i).unwrap_or(&0) as i64;
            let mut s = a - b - borrow;
            if s < 0 {
                s += 1 << 32;
                borrow = 1;
            } else {
                borrow = 0;
            }
            limbs.push(s as u32);
        }
        let mut r = Big { limbs };
        r.normalize();
        r
    }
    fn rem(&self, m: &Big) -> Big {
        use core::cmp::Ordering;
        if self.cmp(m) == Ordering::Less {
            return Big { limbs: self.limbs.clone() };
        }
        // schoolbook long division by repeated shift-subtract.
        let mut rem = Big { limbs: alloc::vec![0] };
        for i in (0..self.bits()).rev() {
            rem = rem.shl1();
            if self.bit(i) {
                rem.limbs[0] |= 1;
            }
            if rem.cmp(m) != Ordering::Less {
                rem = rem.sub(m);
            }
        }
        rem
    }
    /// Schoolbook multiply (O(limbs^2)), producing the full double-width product.
    fn mul(&self, o: &Big) -> Big {
        let mut limbs = alloc::vec![0u32; self.limbs.len() + o.limbs.len() + 1];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry = 0u64;
            for (j, &b) in o.limbs.iter().enumerate() {
                let cur = limbs[i + j] as u64 + a as u64 * b as u64 + carry;
                limbs[i + j] = cur as u32;
                carry = cur >> 32;
            }
            // propagate the final carry up the high limbs
            let mut k = i + o.limbs.len();
            while carry != 0 {
                let cur = limbs[k] as u64 + carry;
                limbs[k] = cur as u32;
                carry = cur >> 32;
                k += 1;
            }
        }
        let mut r = Big { limbs };
        r.normalize();
        r
    }
    fn mulmod(&self, o: &Big, m: &Big) -> Big {
        // full product, then a single reduction — O(bits^2), not O(bits^3).
        self.mul(o).rem(m)
    }
}
