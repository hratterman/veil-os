//! X25519 ECDH over Curve25519 (RFC 7748), ported from the public-domain
//! TweetNaCl `crypto_scalarmult` (16 x 16-bit limbs, Montgomery ladder).
//! no_std, pure integer math.

type Gf = [i64; 16];

const GF0: Gf = [0; 16];
const _121665: Gf = [0xDB41, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

fn car25519(o: &mut Gf) {
    for i in 0..16 {
        o[i] += 1 << 16;
        let c = o[i] >> 16;
        if i < 15 {
            o[i + 1] += c - 1;
        } else {
            // wrap: o[0] += (c-1) + 37*(c-1) = 38*(c-1)  (2^256 ≡ 38 mod p)
            o[0] += 38 * (c - 1);
        }
        o[i] -= c << 16;
    }
}

fn sel25519(p: &mut Gf, q: &mut Gf, b: i64) {
    let c = !(b - 1);
    for i in 0..16 {
        let t = c & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}

fn pack25519(o: &mut [u8; 32], n: &Gf) {
    let mut t: Gf = *n;
    car25519(&mut t);
    car25519(&mut t);
    car25519(&mut t);
    for _ in 0..2 {
        let mut m: Gf = GF0;
        m[0] = t[0] - 0xffed;
        for i in 1..15 {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        let b = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        sel25519(&mut t, &mut m, 1 - b);
    }
    for i in 0..16 {
        o[2 * i] = (t[i] & 0xff) as u8;
        o[2 * i + 1] = (t[i] >> 8) as u8;
    }
}

fn unpack25519(o: &mut Gf, n: &[u8; 32]) {
    for i in 0..16 {
        o[i] = n[2 * i] as i64 + ((n[2 * i + 1] as i64) << 8);
    }
    o[15] &= 0x7fff;
}

fn add(o: &mut Gf, a: &Gf, b: &Gf) {
    for i in 0..16 {
        o[i] = a[i] + b[i];
    }
}

fn sub(o: &mut Gf, a: &Gf, b: &Gf) {
    for i in 0..16 {
        o[i] = a[i] - b[i];
    }
}

fn mul(o: &mut Gf, a: &Gf, b: &Gf) {
    let mut t = [0i64; 31];
    for i in 0..16 {
        for j in 0..16 {
            t[i + j] += a[i] * b[j];
        }
    }
    for i in 0..15 {
        t[i] += 38 * t[i + 16];
    }
    o[..16].copy_from_slice(&t[..16]);
    car25519(o);
    car25519(o);
}

fn sq(o: &mut Gf, a: &Gf) {
    let c = *a;
    mul(o, &c, &c);
}

fn inv25519(o: &mut Gf, i: &Gf) {
    let mut c: Gf = *i;
    for a in (0..=253).rev() {
        let cc = c;
        sq(&mut c, &cc);
        if a != 2 && a != 4 {
            let cc = c;
            mul(&mut c, &cc, i);
        }
    }
    *o = c;
}

/// q = scalar * point (both 32-byte little-endian u-coordinates).
pub fn x25519(scalar: &[u8; 32], point: &[u8; 32]) -> [u8; 32] {
    let mut z = *scalar;
    z[31] = (z[31] & 127) | 64;
    z[0] &= 248;

    let mut x: Gf = GF0;
    unpack25519(&mut x, point);

    let mut a: Gf = GF0;
    let mut b: Gf = x;
    let mut c: Gf = GF0;
    let mut d: Gf = GF0;
    let mut e: Gf = GF0;
    let mut f: Gf = GF0;
    a[0] = 1;
    d[0] = 1;

    for i in (0..=254).rev() {
        let r = ((z[i >> 3] >> (i & 7)) & 1) as i64;
        sel25519(&mut a, &mut b, r);
        sel25519(&mut c, &mut d, r);
        add(&mut e, &a, &c);                       // e = a + c
        { let t = a; sub(&mut a, &t, &c); }        // a = a - c
        add(&mut c, &b, &d);                       // c = b + d
        { let t = b; sub(&mut b, &t, &d); }        // b = b - d
        sq(&mut d, &e);                            // d = e^2
        sq(&mut f, &a);                            // f = a^2
        { let t = a; mul(&mut a, &c, &t); }        // a = c * a
        { let t = c; mul(&mut c, &b, &e); let _ = t; } // c = b * e
        add(&mut e, &a, &c);                       // e = a + c
        { let t = a; sub(&mut a, &t, &c); }        // a = a - c
        sq(&mut b, &a);                            // b = a^2
        sub(&mut c, &d, &f);                       // c = d - f
        { let t = c; mul(&mut a, &t, &_121665); }  // a = c * 121665
        { let t = a; add(&mut a, &t, &d); }        // a = a + d
        { let t = c; mul(&mut c, &t, &a); }        // c = c * a
        mul(&mut a, &d, &f);                       // a = d * f
        mul(&mut d, &b, &x);                       // d = b * x
        sq(&mut b, &e);                            // b = e^2
        sel25519(&mut a, &mut b, r);
        sel25519(&mut c, &mut d, r);
    }

    { let t = c; inv25519(&mut c, &t); }           // c = 1/c
    let mut res: Gf = GF0;
    mul(&mut res, &a, &c);                          // res = a * (1/c)
    let mut out = [0u8; 32];
    pack25519(&mut out, &res);
    out
}

pub fn x25519_base(scalar: &[u8; 32]) -> [u8; 32] {
    let mut base = [0u8; 32];
    base[0] = 9;
    x25519(scalar, &base)
}
