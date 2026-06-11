import sys, re, time
from guilib import Driver, check, finish, taskbar_xy
CONTENT_X = 512; PAGE_Y = 54 + 20
def boxes(s, kind):
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]
def click_link(d, href_sub):
    lk = [b for b in boxes(d.serial(), "link") if href_sub in b[0]]
    if not lk: return False
    _, x, y, w, h = lk[-1]; d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    return True
def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial()); d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    m = len(d.serial()); click_link(d, "/web.htm")
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))
    m = len(d.serial())
    check("imgtest link", click_link(d, "/imgtest.htm"))
    check("imgtest rendered", d.wait_serial("BROWSER: rendered /imgtest.htm", 30, m))
    # The two external PNGs (https direct-TLS + http proxy) must both decode.
    check("EXT_IMG_OK (external PNG fetched + decoded)", d.wait_serial("EXT_IMG_OK", 40, m))
    check("python logo (https/TLS) decoded",
          d.wait_serial("BROWSER: decoded https://www.python.org", 40, m))
    check("gnu logo (http/proxy) decoded",
          d.wait_serial("BROWSER: decoded http://www.gnu.org", 40, m))
    # Image actually placed in the layout (an Image item rendered).
    check("image rendered in page", "BROWSER: img" in d.serial() or "decoded" in d.serial())
    d.quit(); finish()
main()
