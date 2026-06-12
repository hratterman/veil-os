#!/usr/bin/env python3
"""M41 step 17 — TLS certificate validation + HSTS.

Boot self-tests (deterministic) prove the from-scratch crypto + the validation
verdicts against embedded openssl-issued certs:
  * RSA_OK    — bignum modexp
  * X509_OK   — parse + expiry + hostname + self-signed checks + a real 2048-bit
                RSA certificate signature verification
  * HSTS_OK   — Strict-Transport-Security recorded and http:// upgraded to https://
Live acceptance: the browser fetches https://henryratterman.com over direct TLS
1.3 and its certificate validates against a bundled Mozilla root CA (CERT_OK).
"""
import sys

from guilib import Driver, check, finish, taskbar_xy

ADDR = (640, 85)  # browser address bar


def k(d, q, down=None):
    if down is None:
        for s in (True, False):
            d.send([{"type": "key", "data": {"down": s, "key": {"type": "qcode", "data": q}}}])
    else:
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": q}}}])


def type_str(d, s):
    for ch in s:
        if ch == ":":
            seq = [("shift", True), ("semicolon", True), ("semicolon", False), ("shift", False)]
        else:
            q = {"/": "slash", ".": "dot"}.get(ch, ch.lower())
            seq = [(q, True), (q, False)]
        for q, dn in seq:
            k(d, q, dn)


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    s = d.serial()
    check("RSA_OK (from-scratch bignum modexp)", "RSA_OK" in s)
    check("X509_OK (parse/expiry/hostname/self-signed + real RSA cert verify)", "X509_OK" in s)
    check("HSTS_OK (record + http->https upgrade)", "HSTS_OK" in s)

    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # Live acceptance: a real CA-signed site validates against a bundled root.
    m = len(d.serial())
    d.click(*ADDR)
    type_str(d, "https://henryratterman.com")
    k(d, "ret")
    check("henryratterman cert validated against a trusted Mozilla root CA",
          d.wait_serial("CERT_OK: henryratterman.com", 40, m))

    d.move(1000, 700)
    d.dump("m41_tls")
    d.quit()
    finish()


main()
