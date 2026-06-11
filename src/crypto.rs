//! From-scratch crypto for TLS 1.3: SHA-256/HMAC, HKDF, ChaCha20-Poly1305,
//! X25519. Each primitive is checked against its RFC test vectors by
//! `selftest()` at boot (emits CRYPTO_OK), so the TLS code builds on a proven
//! base. no_std, no external crates.

pub mod chacha20;
pub mod hkdf;
pub mod sha256;
pub mod x25519;

use crate::kprintln;
use alloc::vec::Vec;

/// Parse an ASCII hex string into bytes (test-vector helper).
pub fn unhex(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let val = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    let mut i = 0;
    while i + 1 < b.len() {
        out.push((val(b[i]) << 4) | val(b[i + 1]));
        i += 2;
    }
    out
}

fn hex_eq(got: &[u8], want_hex: &str) -> bool {
    got == unhex(want_hex).as_slice()
}

fn tohex(b: &[u8]) -> alloc::string::String {
    let mut s = alloc::string::String::new();
    for &x in b {
        s.push(char::from_digit((x >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((x & 15) as u32, 16).unwrap());
    }
    s
}

/// Run all primitive vectors. Returns true iff every one passes.
pub fn selftest() -> bool {
    let mut ok = true;
    let mut check = |name: &str, pass: bool| {
        if !pass {
            ok = false;
            kprintln!("CRYPTO FAIL: {name}");
        }
    };

    // SHA-256("abc")
    check("sha256", hex_eq(&sha256::sha256(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));

    // HMAC-SHA256, RFC 4231 test case 1.
    let key = [0x0bu8; 20];
    check("hmac", hex_eq(&sha256::hmac_sha256(&key, b"Hi There"),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"));

    // HKDF, RFC 5869 test case 1.
    let ikm = [0x0bu8; 22];
    let salt = unhex("000102030405060708090a0b0c");
    let info = unhex("f0f1f2f3f4f5f6f7f8f9");
    let prk = hkdf::hkdf_extract(&salt, &ikm);
    check("hkdf-extract", hex_eq(&prk,
        "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"));
    let okm = hkdf::hkdf_expand(&prk, &info, 42);
    check("hkdf-expand", hex_eq(&okm,
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"));

    // ChaCha20-Poly1305 AEAD, RFC 8439 section 2.8.2.
    let aead_key = unhex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
    let nonce_v = unhex("070000004041424344454647");
    let aad = unhex("50515253c0c1c2c3c4c5c6c7");
    let pt = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let mut k = [0u8; 32];
    k.copy_from_slice(&aead_key);
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&nonce_v);
    let ct = chacha20::chacha20poly1305_encrypt(&k, &nonce, &aad, pt);
    let tag = &ct[ct.len() - 16..];
    check("chacha20poly1305-tag", hex_eq(tag, "1ae10b594f09e26a7e902ecbd0600691"));
    let rt = chacha20::chacha20poly1305_decrypt(&k, &nonce, &aad, &ct);
    check("chacha20poly1305-roundtrip", rt.as_deref() == Some(&pt[..]));

    // X25519, RFC 7748 section 6.1 (Alice's keypair from her private scalar).
    let alice_sk = unhex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
    let mut sk = [0u8; 32];
    sk.copy_from_slice(&alice_sk);
    check("x25519-base", hex_eq(&x25519::x25519_base(&sk),
        "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a"));

    // X25519 scalarmult, RFC 7748 section 5.2 test vector.
    let scal = unhex("a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4");
    let upt = unhex("e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c");
    let mut s2 = [0u8; 32];
    let mut u2 = [0u8; 32];
    s2.copy_from_slice(&scal);
    u2.copy_from_slice(&upt);
    check("x25519-mult", hex_eq(&x25519::x25519(&s2, &u2),
        "c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552"));

    if ok {
        kprintln!("CRYPTO_OK");
    }
    ok
}
