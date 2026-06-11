import sys, time
from guilib import Driver, check, finish
LISP_BTN = (70 + 9 * 78 + 36, 768 - 20)   # no-NIC taskbar: lisp idx 9
SHIFTED = {'(': '9', ')': '0', '*': '8', '+': 'equal', '?': 'slash', '"': "apostrophe"}
BASE = {' ': 'spc', '-': 'minus', '=': 'equal', '.': 'dot', '"': "apostrophe"}
def k(d, qc, shift=False):
    ev = []
    if shift: ev.append({"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "shift"}}})
    for dn in (True, False):
        ev.append({"type": "key", "data": {"down": dn, "key": {"type": "qcode", "data": qc}}})
    if shift: ev.append({"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "shift"}}})
    d.send(ev)
def typ(d, s):
    for c in s:
        if c == '"': k(d, "apostrophe", True)   # shift-' = "
        elif c in SHIFTED: k(d, SHIFTED[c], True)
        elif c in BASE: k(d, BASE[c])
        elif c.isdigit(): k(d, c)
        elif c.isalpha(): k(d, c.lower())
        else: k(d, "spc")
        time.sleep(0.01)
    k(d, "ret")
def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial()); d.click(*LISP_BTN)
    check("lisp launched", d.wait_serial("LISP: window open", 5, m))
    check("LISP_IO_OK (self-test write+read+list)", d.wait_serial("LISP_IO_OK", 6, m))
    # Interactive: write a file, read it back.
    m = len(d.serial()); typ(d, '(write-file "HELLO.TXT" "world")')
    check("write-file returns #t", d.wait_serial('=> #t', 5, m))
    # The typ() helper lowercases letters; FAT lookup is case-insensitive, so
    # "hello.txt" resolves to the HELLO.TXT we wrote. Match the echoed input.
    m = len(d.serial()); typ(d, '(read-file "HELLO.TXT")')
    check("read-file returns the contents", d.wait_serial('LISP_EVAL: (read-file "hello.txt") => "world"', 5, m))
    # Missing file -> #f.
    m = len(d.serial()); typ(d, '(read-file "NOPE.TXT")')
    check("read-file of missing file => #f", d.wait_serial('LISP_EVAL: (read-file "nope.txt") => #f', 5, m))
    # list-files includes what we wrote (stored uppercase on FAT).
    m = len(d.serial()); typ(d, "(list-files)")
    check("list-files includes HELLO.TXT", d.wait_serial('HELLO.TXT', 5, m))
    d.quit(); finish()
main()
