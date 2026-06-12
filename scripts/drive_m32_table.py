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
    m=len(d.serial())
    lk=[b for b in boxes(d.serial(),"link") if b[0]=="/changes.htm"][-1]
    _,x,y,w,h=lk
    d.click(CONTENT_X+x+w//2, PAGE_Y+y+h//2)
    check("changelog (with table) rendered", d.wait_serial("BROWSER: rendered /changes.htm",20,m))
    check("TABLE_OK emitted", d.wait_serial("TABLE_OK",5,m))
    d.move(1000,700); time.sleep(0.3); img=d.dump("m32_table")
    # table border colour 0xff586068 -> (88,96,104). Scan the content area.
    LINE=(88,96,104)
    found=any(img.at(x,y)==LINE for y in range(80,640,3) for x in range(514,990,4))
    check("table borders rendered", found)
    d.quit(); finish()
main()
