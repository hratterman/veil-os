import sys, re, time
from guilib import Driver, check, finish, taskbar_xy
CONTENT_X=512; PAGE_Y=54+20
def boxes(s,kind):
    return [(m[0],int(m[1]),int(m[2]),int(m[3]),int(m[4])) for m in re.findall(rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]
def main():
    d=Driver(sys.argv[1],sys.argv[2],sys.argv[3])
    m=len(d.serial()); d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'",5,m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -",40))
    # navigate forward: click the /news.htm nav link
    m=len(d.serial())
    lk=[b for b in boxes(d.serial(),"link") if b[0]=="/news.htm"]
    check("news link present", len(lk)>0)
    if not lk: finish(); return
    _,x,y,w,h=lk[-1]
    d.click(CONTENT_X+x+w//2, PAGE_Y+y+h//2)
    check("navigated forward to news", d.wait_serial("BROWSER: rendered /news.htm",20,m))
    # now click the back button (content x<18, top bar)
    m=len(d.serial())
    d.click(512+8, 54+10)   # back button zone
    check("back to previous (/)", d.wait_serial("BROWSER: back to /",20,m))
    check("HISTORY_OK emitted", d.wait_serial("HISTORY_OK",5,m))
    # verify the back button visual zone exists (bluish when history)
    d.move(1000,700); d.dump("m32_history")
    # also test Backspace -> back (navigate forward again, then backspace)
    m=len(d.serial())
    lk=[b for b in boxes(d.serial(),"link") if b[0]=="/wiki.htm"][-1]
    _,x,y,w,h=lk
    d.click(CONTENT_X+x+w//2, PAGE_Y+y+h//2)
    check("forward to wiki", d.wait_serial("BROWSER: rendered /wiki.htm",20,m))
    m=len(d.serial())
    for dn in (True,False): d.send([{"type":"key","data":{"down":dn,"key":{"type":"qcode","data":"backspace"}}}])
    check("Backspace navigates back", d.wait_serial("BROWSER: back to",5,m))
    d.quit(); finish()
main()
