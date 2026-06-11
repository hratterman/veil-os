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
    check("cssvar link", click_link(d, "/cssvar.htm"))
    check("cssvar rendered", d.wait_serial("BROWSER: rendered /cssvar.htm", 20, m))
    check("CSS_VAR_OK (var() substituted)", d.wait_serial("CSS_VAR_OK", 6, m))
    # The .vbar background is var(--brand) = #2a7e3b = rgb(42,126,59). Scan a
    # vertical strip of the content area for that exact colour (the bar fills a
    # solid region behind its text + padding).
    d.move(1000, 700); img = d.dump("m34_cssvar")
    brand = (0x2a, 0x7e, 0x3b)
    # Sample a 2D region inside the content (past the body's 20px margin).
    found = [img.at(x, y)
             for y in range(PAGE_Y, PAGE_Y + 380, 3)
             for x in range(CONTENT_X + 40, CONTENT_X + 400, 20)]
    check("var(--brand) background rendered (green bar present)",
          brand in found, f"sampled colours: {sorted(set(found))[:10]}")
    d.quit(); finish()
main()
