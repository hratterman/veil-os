//! M41 step 17: X.509 certificate parsing + chain validation for the TLS client.
//!
//! From-scratch DER/ASN.1 parsing (no crates). `validate(chain, host, now)`
//! checks, in order: the leaf's notBefore..notAfter window (expiry), the
//! hostname against the SAN dNSNames (or CN, wildcards included), and the trust
//! chain — each issuer DN must equal the next subject DN, presented RSA links
//! are cryptographically verified (`crate::rsa`), and the chain must terminate
//! at a **bundled Mozilla root** (the top issuer's Subject-DN SHA-256 is in
//! `x509_roots.rs`, generated from certifi). Self-signed leaves and unknown
//! roots fail. ECDSA links are checked structurally (DN chaining) — documented.

use alloc::string::String;
use alloc::vec::Vec;
use crate::crypto::sha256::sha256;

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum CertStatus {
    Ok,
    Expired,
    NotYetValid,
    HostnameMismatch,
    SelfSigned,
    Untrusted,
    ParseError,
}

impl CertStatus {
    pub fn reason(self) -> &'static str {
        match self {
            CertStatus::Ok => "valid",
            CertStatus::Expired => "the certificate has expired",
            CertStatus::NotYetValid => "the certificate is not yet valid",
            CertStatus::HostnameMismatch => "the certificate is for a different host",
            CertStatus::SelfSigned => "the certificate is self-signed (not from a trusted CA)",
            CertStatus::Untrusted => "the issuer is not a trusted certificate authority",
            CertStatus::ParseError => "the certificate could not be parsed",
        }
    }
    pub fn ok(self) -> bool {
        self == CertStatus::Ok
    }
}

struct Cert {
    tbs: Vec<u8>,
    issuer_dn: Vec<u8>,
    subject_dn: Vec<u8>,
    cn: String,
    sans: Vec<String>,
    not_before: i64,
    not_after: i64,
    spki: Vec<u8>,
    sig_rsa: bool,
    signature: Vec<u8>,
}

// --- DER TLV helpers (offset-based, no lifetimes to fight) --------------------

/// (tag, content-start, content-end, element-end) of the TLV at `off`.
fn tlv(b: &[u8], off: usize) -> Option<(u8, usize, usize, usize)> {
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
    let cs = off + hdr;
    let ce = cs.checked_add(len)?;
    if ce > b.len() {
        return None;
    }
    Some((tag, cs, ce, ce))
}

const OID_CN: &[u8] = &[0x55, 0x04, 0x03];
const OID_SAN: &[u8] = &[0x55, 0x1d, 0x11];
const OID_RSA_SHA256: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b];

fn parse(der: &[u8]) -> Option<Cert> {
    let (_, cs, ce, _) = tlv(der, 0)?; // outer SEQUENCE
    let body = &der[cs..ce];
    // TBSCertificate (keep raw element bytes for signature verification).
    let (_, tcs, tce, after_tbs) = tlv(body, 0)?;
    let tbs = body[0..after_tbs].to_vec(); // whole TBS element (header+content)
    // signatureAlgorithm
    let (_, acs, ace, after_alg) = tlv(body, after_tbs)?;
    let sig_rsa = window(&body[acs..ace], OID_RSA_SHA256);
    // signatureValue BIT STRING
    let (sbt, scs, sce, _) = tlv(body, after_alg)?;
    if sbt != 0x03 || sce <= scs {
        return None;
    }
    let signature = body[scs + 1..sce].to_vec(); // drop unused-bits byte

    // Inside the TBS.
    let tb = &body[tcs..tce];
    let mut p = 0;
    // optional [0] version
    if *tb.get(0)? == 0xA0 {
        let (_, _, _, e) = tlv(tb, 0)?;
        p = e;
    }
    let (_, _, _, e) = tlv(tb, p)?; // serialNumber
    p = e;
    let (_, _, _, e) = tlv(tb, p)?; // signature alg
    p = e;
    let (_, ics, ice, e) = tlv(tb, p)?; // issuer Name (content)
    let issuer_dn = tb[p..e].to_vec(); // raw issuer Name element
    let _ = (ics, ice);
    p = e;
    let (_, vcs, vce, e) = tlv(tb, p)?; // validity
    p = e;
    let subj_start = p;
    let (_, scs2, sce2, e) = tlv(tb, p)?; // subject Name (content)
    let subject_dn = tb[subj_start..e].to_vec();
    let _ = (scs2, sce2);
    p = e;
    let spki_start = p;
    let (_, _, _, e) = tlv(tb, p)?; // SubjectPublicKeyInfo
    let spki = tb[spki_start..e].to_vec();

    // validity -> times
    let v = &tb[vcs..vce];
    let (t1, n1s, n1e, after1) = tlv(v, 0)?;
    let (t2, n2s, n2e, _) = tlv(v, after1)?;
    let not_before = parse_time(t1, &v[n1s..n1e])?;
    let not_after = parse_time(t2, &v[n2s..n2e])?;

    let cn = find_cn(&subject_dn).unwrap_or_default();
    let sans = find_sans(der);

    Some(Cert { tbs, issuer_dn, subject_dn, cn, sans, not_before, not_after, spki, sig_rsa, signature })
}

fn window(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// CN from a Name element (its own SEQ header included).
fn find_cn(name: &[u8]) -> Option<String> {
    let (_, cs, ce, _) = tlv(name, 0)?;
    let rdns = &name[cs..ce];
    let mut p = 0;
    while p < rdns.len() {
        let (t, scs, sce, e) = tlv(rdns, p)?;
        p = e;
        if t != 0x31 {
            continue;
        }
        let set = &rdns[scs..sce];
        let mut q = 0;
        while q < set.len() {
            let (_, acs, ace, ae) = tlv(set, q)?;
            q = ae;
            let atv = &set[acs..ace];
            let (_, ocs, oce, after_oid) = tlv(atv, 0)?;
            if atv[ocs..oce] == *OID_CN {
                let (vt, vcs, vce, _) = tlv(atv, after_oid)?;
                if matches!(vt, 0x0c | 0x13 | 0x16) {
                    return Some(String::from_utf8_lossy(&atv[vcs..vce]).into_owned());
                }
            }
        }
    }
    None
}

/// Scan the cert for the SAN extension and pull out dNSNames.
fn find_sans(der: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    // OID element for SAN: 06 03 55 1d 11
    let needle = [0x06u8, 0x03, OID_SAN[0], OID_SAN[1], OID_SAN[2]];
    let mut i = 0;
    while i + needle.len() <= der.len() {
        if der[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            if der.get(j) == Some(&0x01) {
                j += 3; // BOOLEAN critical
            }
            if der.get(j) == Some(&0x04) {
                if let Some((_, ocs, oce, _)) = tlv(der, j) {
                    let octet = &der[ocs..oce];
                    if let Some((0x30, scs, sce, _)) = tlv(octet, 0) {
                        let names = &octet[scs..sce];
                        let mut p = 0;
                        while p < names.len() {
                            if let Some((tag, vcs, vce, e)) = tlv(names, p) {
                                if tag == 0x82 {
                                    out.push(String::from_utf8_lossy(&names[vcs..vce]).into_owned());
                                }
                                p = e;
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
            break;
        }
        i += 1;
    }
    out
}

fn parse_time(tag: u8, b: &[u8]) -> Option<i64> {
    let digits: Vec<u8> = b.iter().copied().take_while(|c| c.is_ascii_digit()).collect();
    let g = |o: usize, n: usize| -> Option<i64> {
        let mut v = 0i64;
        for k in 0..n {
            v = v * 10 + (*digits.get(o + k)? - b'0') as i64;
        }
        Some(v)
    };
    let (year, off) = if tag == 0x17 {
        let yy = g(0, 2)?;
        (if yy < 50 { 2000 + yy } else { 1900 + yy }, 2)
    } else if tag == 0x18 {
        (g(0, 4)?, 4)
    } else {
        return None;
    };
    let (mon, day, hh, mm) = (g(off, 2)?, g(off + 2, 2)?, g(off + 4, 2)?, g(off + 6, 2)?);
    let ss = g(off + 8, 2).unwrap_or(0);
    Some(civil_to_unix(year, mon, day, hh, mm, ss))
}

fn civil_to_unix(y: i64, m: i64, d: i64, hh: i64, mm: i64, ss: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146097 + doe - 719468) * 86400 + hh * 3600 + mm * 60 + ss
}

/// Validate the leaf-first `chain` for `host` at unix time `now` (0 = skip date
/// checks when the clock isn't synced).
pub fn validate(chain: &[Vec<u8>], host: &str, now: i64) -> CertStatus {
    if chain.is_empty() {
        return CertStatus::ParseError;
    }
    let certs: Vec<Cert> = chain.iter().filter_map(|c| parse(c)).collect();
    if certs.is_empty() {
        return CertStatus::ParseError;
    }
    let leaf = &certs[0];

    if now > 0 {
        if now > leaf.not_after {
            return CertStatus::Expired;
        }
        if now < leaf.not_before {
            return CertStatus::NotYetValid;
        }
    }
    if !host_matches(leaf, host) {
        return CertStatus::HostnameMismatch;
    }

    // A self-signed leaf (issuer == subject) that isn't a bundled root is the
    // classic "self-signed certificate" case.
    if leaf.issuer_dn == leaf.subject_dn && !is_trusted_root(&leaf.subject_dn) {
        return CertStatus::SelfSigned;
    }

    // Walk the presented chain: DN linkage + RSA signature verification.
    for i in 0..certs.len() {
        let c = &certs[i];
        if let Some(parent) = certs.get(i + 1) {
            if c.issuer_dn != parent.subject_dn {
                return CertStatus::Untrusted;
            }
            if c.sig_rsa && crate::rsa::is_rsa_spki(&parent.spki)
                && !crate::rsa::verify_pkcs1_sha256(&parent.spki, &c.tbs, &c.signature)
            {
                crate::kprintln!("X509: RSA signature on a chain link did not verify");
                return CertStatus::Untrusted;
            }
        } else {
            // Top of the presented chain: its issuer must be a bundled root.
            if !is_trusted_root(&c.issuer_dn) {
                return CertStatus::Untrusted;
            }
        }
    }
    CertStatus::Ok
}

fn host_matches(leaf: &Cert, host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    leaf.sans.iter().any(|s| dns_match(&s.to_ascii_lowercase(), &h))
        || (!leaf.cn.is_empty() && dns_match(&leaf.cn.to_ascii_lowercase(), &h))
}

fn dns_match(pat: &str, host: &str) -> bool {
    if let Some(suffix) = pat.strip_prefix("*.") {
        if let Some((_, rest)) = host.split_once('.') {
            return rest == suffix;
        }
        return false;
    }
    pat == host
}

fn is_trusted_root(subject_dn: &[u8]) -> bool {
    let h = sha256(subject_dn);
    TRUSTED_ROOT_DN_HASHES.iter().any(|r| r == &h)
}

/// Boot self-test: validate embedded openssl-issued certs (self-signed +
/// expired), check hostname matching, and verify a real 2048-bit RSA signature.
pub fn selftest() {
    // A fixed "now": 2026-06-13 (after the self-signed cert's notBefore, before
    // its notAfter, and well after the expired cert's notAfter of 2020-02-01).
    const NOW: i64 = 1_781_308_800;
    let ss = SELF_SIGNED_CERT.to_vec();
    let ex = EXPIRED_CERT.to_vec();

    let r_self = validate(&[ss.clone()], "veiltest.local", NOW);
    let r_exp = validate(&[ex.clone()], "expired.local", NOW);
    let r_host = validate(&[ss.clone()], "wrong.host", NOW);

    // Verify the self-signed cert's RSA signature against its own SPKI (a real
    // 2048-bit PKCS#1 v1.5 / SHA-256 verification through the from-scratch modexp).
    let rsa_ok = parse(&ss)
        .map(|c| crate::rsa::verify_pkcs1_sha256(&c.spki, &c.tbs, &c.signature))
        .unwrap_or(false);

    crate::kprintln!(
        "X509: self-signed={:?} expired={:?} wrong-host={:?} rsa-verify={}",
        r_self, r_exp, r_host, rsa_ok
    );
    if r_self == CertStatus::SelfSigned
        && r_exp == CertStatus::Expired
        && r_host == CertStatus::HostnameMismatch
        && rsa_ok
    {
        crate::kprintln!("X509_OK: parse + expiry + hostname + self-signed checks + real RSA cert verify");
    } else {
        crate::kprintln!("X509_FAIL: self={r_self:?} exp={r_exp:?} host={r_host:?} rsa={rsa_ok}");
    }
}

include!("x509_roots.rs");
include!("x509_test.rs");
