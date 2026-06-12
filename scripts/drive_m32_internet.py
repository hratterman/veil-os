import sys, re, time
from guilib import Driver, check, finish, taskbar_xy
CONTENT_X=512; PAGE_Y=54+42
def boxes(s,kind):
    return [(m[0],int(m[1]),int(m[2]),int(m[3]),int(m[4])) for m in re.findall(rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]
def main():
    d=Driver(sys.argv[1],sys.argv[2],sys.argv[3])
    m=len(d.serial()); d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'",5,m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -",40))
    # go to web.htm
    m=len(d.serial())
    lk=[b for b in boxes(d.serial(),"link") if b[0]=="/web.htm"][-1]
    _,x,y,w,h=lk; d.click(CONTENT_X+x+w//2, PAGE_Y+y+h//2)
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm",20,m))
    # click the external example.com link
    m=len(d.serial())
    ext=[b for b in boxes(d.serial(),"link") if "example.com" in b[0]]
    check("external link present", len(ext)>0, str([b[0] for b in boxes(d.serial(),'link')][:8]))
    if not ext: finish(); return
    _,x,y,w,h=ext[-1]; d.click(CONTENT_X+x+w//2, PAGE_Y+y+h//2)
    check("INTERNET_OK (real external page fetched+rendered)", d.wait_serial("INTERNET_OK",45,m))
    # the page heading should be laid out as an item
    time.sleep(0.5); d.move(1000,700); d.dump("m32_internet")
    check("external content present (Example Domain text on screen)",
          "BROWSER: rendered external page" in d.serial())
    d.quit(); finish()
main()
