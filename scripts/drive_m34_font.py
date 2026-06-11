import sys, re, time
from guilib import Driver, check, finish
CONTENT_X = 512; PAGE_Y = 54 + 20
def boxes(s, kind):
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]
def click_link(d, sub):
    lk = [b for b in boxes(d.serial(), "link") if sub in b[0]]
    if not lk: return False
    _, x, y, w, h = lk[-1]; d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    return True
def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    check("FONTS_OK (boot self-test)", d.wait_serial("FONTS_OK", 8))
    m = len(d.serial()); d.click(262, 768 - 20)
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    m = len(d.serial()); click_link(d, "/web.htm")
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))
    m = len(d.serial())
    check("fonttest link", click_link(d, "/fonttest.htm"))
    check("fonttest rendered", d.wait_serial("BROWSER: rendered /fonttest.htm", 20, m))
    time.sleep(0.6); d.move(1000, 700); d.dump("m34_fonts")
    d.quit(); finish()
main()
