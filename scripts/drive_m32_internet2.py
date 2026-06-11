import sys, re, time
from guilib import Driver, check, finish, taskbar_xy
CONTENT_X=512; PAGE_Y=54+20
def boxes(s,kind):
    return [(m[0],int(m[1]),int(m[2]),int(m[3]),int(m[4])) for m in re.findall(rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]
def link_for(d, sub):
    return [b for b in boxes(d.serial(),"link") if sub in b[0]]
def visit(d, sub, host):
    """Click the external link whose href contains `sub`, return after render.
    INTERNET_OK/`rendered external page` latch after the first external page,
    so wait on the generic per-page render line instead."""
    m=len(d.serial())
    lk=link_for(d, sub)
    if not lk:
        return None
    _,x,y,w,h=lk[-1]
    d.click(CONTENT_X+x+w//2, PAGE_Y+y+h//2)
    ok = d.wait_serial(f"BROWSER: rendered {host}", 45, m)
    return ok
def goto_web(d):
    m=len(d.serial())
    lk=[b for b in boxes(d.serial(),"link") if b[0]=="/web.htm"]
    if not lk: return False
    _,x,y,w,h=lk[-1]; d.click(CONTENT_X+x+w//2, PAGE_Y+y+h//2)
    return d.wait_serial("BROWSER: rendered /web.htm",20,m)
def main():
    d=Driver(sys.argv[1],sys.argv[2],sys.argv[3])
    m=len(d.serial()); d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'",5,m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -",40))
    check("web.htm rendered", goto_web(d))
    # Visit neverssl (pure HTTP) and Hacker News (HTTPS via proxy).
    for sub,host,label in [("neverssl","http://neverssl.com","neverssl.com (HTTP)"),
                           ("news.ycombinator","https://news.ycombinator.com","Hacker News (HTTPS)")]:
        ok = visit(d, sub, host)
        check(f"fetched+rendered {label}", bool(ok))
        time.sleep(0.3)
        # back to web.htm for the next link (Backspace navigates back)
        m=len(d.serial())
        d.q.cmd("input-send-event", events=[{"type":"key","data":{"down":True,"key":{"type":"qcode","data":"backspace"}}},
                                            {"type":"key","data":{"down":False,"key":{"type":"qcode","data":"backspace"}}}])
        d.wait_serial("BROWSER: rendered /web.htm",20,m)
        time.sleep(0.3)
    d.move(1000,700); d.dump("m32_internet2")
    d.quit(); finish()
main()
