import sys, time
from guilib import Driver, check, finish
def key(d,qc):
    for dn in (True,False): d.send([{"type":"key","data":{"down":dn,"key":{"type":"qcode","data":qc}}}])
def main():
    d=Driver(sys.argv[1],sys.argv[2],sys.argv[3])
    m=len(d.serial()); d.click(262,768-20)
    check("browser launched", d.wait_serial("WM: launch 'browser'",5,m))
    check("index rendered (tall page)", d.wait_serial("BROWSER: rendered / -",40))
    # keyboard scroll -> SCROLL_OK
    m=len(d.serial())
    for _ in range(3): key(d,"pgdn")
    check("SCROLL_OK (scrolled past top via keys)", d.wait_serial("SCROLL_OK",6,m))
    # scrollbar drawn: dump, check thumb color present on right edge
    d.move(1000,700); img=d.dump("m32_scroll")
    # browser window (510,30) content 480x620; right edge x ~ 510+2+480-2=988
    col=[img.at(990, y) for y in range(80, 660, 6)]
    thumb=(0x70,0x90,0xb0); track=(0x20,0x28,0x30)
    check("scrollbar thumb visible on right edge", any(p==thumb for p in col), str(set(col)))
    check("scrollbar track visible", any(p==track for p in col))
    # mouse wheel: inject REL_WHEEL up, scroll should move toward top
    m=len(d.serial())
    d.q.cmd("input-send-event", events=[{"type":"btn","data":{"down":True,"button":"wheel-up"}}])
    d.q.cmd("input-send-event", events=[{"type":"btn","data":{"down":False,"button":"wheel-up"}}])
    time.sleep(0.3)
    check("mouse wheel scrolls (serial scroll y=)", d.wait_serial("BROWSER: scroll y=",4,m))
    d.quit(); finish()
main()
