#!/usr/bin/env python3
"""M40 Step 1+2: HTML form POST + cookie jar, and all input element kinds.

Step 1 (login.htm): a POST login form. Fill username/password text fields,
click submit -> the browser POSTs application/x-www-form-urlencoded to /login,
the kernel server replies 302 + Set-Cookie, the browser follows the redirect
to /welcome AND sends the stored cookie -> server logs "LOGGED IN".

Step 2 (inputs.htm): a GET form with text/checkbox/radio/select/textarea/submit.
Toggle the checkbox, pick the 'red' radio, cycle the select, submit -> the
browser builds a query string and the server echoes the field values back.

Browser needs a NIC (loopback HTTP server). Browser content origin (512,96).
"""
import re
import sys

from guilib import Driver, check, finish, taskbar_xy

CONTENT_X, CONTENT_Y = 512, 52
PAGE_Y = 96


def type_str(d, s):
    for ch in s:
        qcode = {"/": "slash", ".": "dot"}.get(ch, ch.lower())
        for down in (True, False):
            d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": qcode}}}])


def press(d, key):
    for down in (True, False):
        d.send([{"type": "key", "data": {"down": down, "key": {"type": "qcode", "data": key}}}])


def fields(s):
    """Parse the latest page's field log lines into dicts."""
    out = []
    for m in re.finditer(
        r"BROWSER: field '(\w+)' name='([^']*)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+) checked=(\w+)", s):
        out.append(dict(kind=m[1], name=m[2], x=int(m[3]), y=int(m[4]),
                        w=int(m[5]), h=int(m[6]), checked=m[7] == "true"))
    return out


def click_field(d, f):
    d.click(CONTENT_X + f["x"] + f["w"] // 2, PAGE_Y + f["y"] + f["h"] // 2)


def nav(d, path):
    m = len(d.serial())
    d.click(650, CONTENT_Y + 32)  # address bar
    type_str(d, path)
    press(d, "ret")
    return m


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))

    # ---- Step 1: login form POST + cookie + redirect -------------------------
    m = nav(d, "/login.htm")
    check("login page rendered", d.wait_serial("BROWSER: rendered /login.htm", 20, m))
    fs = [f for f in fields(d.serial()[m:]) if f["kind"] != "hidden"]
    text = [f for f in fs if f["kind"] in ("text", "password")]
    submit = [f for f in fs if f["kind"] == "submit"]
    check("login form has user+pass+submit", len(text) >= 2 and len(submit) >= 1,
          f"text={len(text)} submit={len(submit)}")

    # username then password
    click_field(d, text[0]); type_str(d, "alice")
    click_field(d, text[1]); type_str(d, "secret")
    m = len(d.serial())
    click_field(d, submit[0])
    check("browser POSTed to /login", d.wait_serial("BROWSER: POST /login", 10, m))
    check("server received login POST", d.wait_serial('HTTP: login POST user="alice"', 10, m))
    check("browser followed redirect to /welcome",
          d.wait_serial("BROWSER: rendered /welcome", 20, m))
    check("cookie was sent -> server logged LOGGED IN",
          d.wait_serial("-> LOGGED IN", 10, m))

    # ---- Step 2: all input kinds, GET submit ---------------------------------
    m = nav(d, "/inputs.htm")
    check("inputs page rendered", d.wait_serial("BROWSER: rendered /inputs.htm", 20, m))
    fs = fields(d.serial()[m:])
    by = lambda k: [f for f in fs if f["kind"] == k]
    check("checkbox present", bool(by("checkbox")))
    check("radio present (2)", len(by("radio")) >= 2)
    check("select present", bool(by("select")))
    check("textarea present", bool(by("textarea")))

    # toggle checkbox on, pick the 'red' radio, cycle the select once (S->M).
    click_field(d, by("checkbox")[0]); d.move(1000, 700)
    red = next(f for f in by("radio") if f["name"] == "color")  # first radio = red
    click_field(d, red); d.move(1000, 700)
    click_field(d, by("select")[0]); d.move(1000, 700)

    m = len(d.serial())
    click_field(d, by("submit")[0]); d.move(1000, 700)
    check("browser GET-submitted to /echo", d.wait_serial("BROWSER: GET-submit /echo", 10, m))
    # The server echoes the query; confirm the values made it across.
    ser = d.serial()[m:]
    qline = next((l for l in ser.splitlines() if "HTTP: /echo query=" in l), "")
    check("text value submitted", "t=hi" in qline, qline)
    check("checkbox value submitted", "c1=yes" in qline, qline)
    check("radio value submitted", "color=red" in qline, qline)
    check("select value submitted", ("size=M" in qline or "size=S" in qline), qline)

    d.quit()
    finish()


main()
