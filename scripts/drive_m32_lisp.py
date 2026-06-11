import sys, time
from guilib import Driver, check, finish
LISP_BTN=(70+9*78+36, 768-20)
SHIFTED={'(' :'9',')':'0','*':'8','+':'equal','?':'slash'}
BASE={' ':'spc','-':'minus','=':'equal'}
def k(d,qc,shift=False):
    ev=[]
    if shift: ev.append({"type":"key","data":{"down":True,"key":{"type":"qcode","data":"shift"}}})
    ev.append({"type":"key","data":{"down":True,"key":{"type":"qcode","data":qc}}})
    ev.append({"type":"key","data":{"down":False,"key":{"type":"qcode","data":qc}}})
    if shift: ev.append({"type":"key","data":{"down":False,"key":{"type":"qcode","data":"shift"}}})
    d.send(ev)
def typ(d,s):
    for c in s:
        if c in SHIFTED: k(d,SHIFTED[c],True)
        elif c in BASE: k(d,BASE[c])
        elif c.isdigit(): k(d,c)
        elif c.isalpha(): k(d,c.lower())
        else: k(d,"spc")
        time.sleep(0.01)
    k(d,"ret")
def main():
    d=Driver(sys.argv[1],sys.argv[2],sys.argv[3])
    m=len(d.serial()); d.click(*LISP_BTN)
    check("lisp launched", d.wait_serial("LISP: window open",5,m))
    check("LISP_OK", d.wait_serial("LISP_OK",8,m))
    typ(d,"(* 6 7)"); time.sleep(0.2)
    typ(d,"(map (lambda (x) (* x x)) (list 1 2 3 4 5))"); time.sleep(0.2)
    typ(d,"(define (sq n) (* n n))"); time.sleep(0.2)
    typ(d,"(sq 9)"); time.sleep(0.2)
    d.move(1000,700); d.dump("m32_lisp")
    d.quit(); finish()
main()
