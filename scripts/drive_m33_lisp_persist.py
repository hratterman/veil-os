import sys, time
from guilib import Driver, check, finish
LISP_BTN = (70 + 9 * 78 + 36, 768 - 20)   # no-NIC taskbar: lisp idx 9
CLOSE = (620, 82)                          # rightmost title-bar X of the Lisp win
SHIFTED = {'(': '9', ')': '0', '*': '8', '+': 'equal', '?': 'slash'}
BASE = {' ': 'spc', '-': 'minus', '=': 'equal'}
def k(d, qc, shift=False):
    ev = []
    if shift: ev.append({"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}})
    for dn in (True, False):
        ev.append({"type": "key", "data": {"down": dn, "key": {"type": "qcode", "data": qc}}})
    if shift: ev.append({"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}})
    d.send(ev)
def typ(d, s):
    for c in s:
        if c in SHIFTED: k(d, SHIFTED[c], True)
        elif c in BASE: k(d, BASE[c])
        elif c.isdigit(): k(d, c)
        elif c.isalpha(): k(d, c.lower())
        else: k(d, "spc")
        time.sleep(0.01)
    k(d, "ret")
def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    # 1. Launch Lisp, define a variable.
    m = len(d.serial()); d.click(*LISP_BTN)
    check("lisp launched", d.wait_serial("LISP: window open", 5, m))
    m = len(d.serial()); typ(d, "(define wibble 1234)")
    check("define evaluated", d.wait_serial("LISP_EVAL: (define wibble 1234) => wibble", 5, m))
    time.sleep(0.3)  # let LISP.TXT write settle
    # 2. Close the REPL window.
    m = len(d.serial()); d.click(*CLOSE)
    check("window closed", d.wait_serial("WM: closed 'lisp'", 5, m))
    time.sleep(0.2)
    # 3. Reopen — a fresh LispState that must restore from LISP.TXT.
    m = len(d.serial()); d.click(*LISP_BTN)
    check("lisp relaunched", d.wait_serial("LISP: window open", 5, m))
    check("env restored from LISP.TXT", d.wait_serial("LISP: restored", 5, m))
    # 4. The variable must still be bound.
    m = len(d.serial()); typ(d, "wibble")
    check("restored variable still bound (wibble => 1234)",
          d.wait_serial("LISP_EVAL: wibble => 1234", 5, m))
    d.quit(); finish()
main()
