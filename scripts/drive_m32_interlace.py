import sys, re, time
from guilib import Driver, check, finish, taskbar_xy
VIEWER_BTN=(70+5*78+36, 768-20)  # no NIC: viewer idx5
CX,CY,CW,CH=222,104,560,460
def key(d,qc):
    for dn in (True,False): d.send([{"type":"key","data":{"down":dn,"key":{"type":"qcode","data":qc}}}])
def region(img):
    out=b""
    for y in range(CY,CY+CH):
        i=(y*img.w+CX)*3; out+=img.px[i:i+CW*3]
    return out
def goto(d,name):
    for _ in range(30):
        last=re.findall(r"VIEWER: showing (\S+\.PNG)", d.serial())
        if last and last[-1]==name: return True
        m=len(d.serial()); key(d,"right"); d.wait_serial("VIEWER: showing ",3,m)
    return False
def main():
    d=Driver(sys.argv[1],sys.argv[2],sys.argv[3])
    m=len(d.serial()); d.click(*taskbar_xy(d, "viewer"))
    check("viewer launched", d.wait_serial("WM: launch 'viewer'",5,m))
    check("first decode", d.wait_serial("VIEWER: showing ",5,m))
    check("plain GRAD.PNG decodes", goto(d,"GRAD.PNG"))
    d.move(1000,700); time.sleep(0.2); plain=region(d.dump("m32_grad"))
    check("interlaced GRADI.PNG decodes", goto(d,"GRADI.PNG"))
    check("INTERLACE_OK emitted", "INTERLACE_OK" in d.serial())
    d.move(1000,700); time.sleep(0.2); inter=region(d.dump("m32_gradi"))
    check("Adam7 deinterlace == plain (identical pixels)", plain==inter,
          f"{sum(1 for a,b in zip(plain,inter) if a!=b)} differing bytes")
    d.quit(); finish()
main()
