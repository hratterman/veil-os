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
    # The acceptance test: load henryratterman.com over direct TLS 1.3.
    m = len(d.serial())
    check("henryratterman link present", click_link(d, "henryratterman.com"))
    check("TLS handshake to henryratterman", d.wait_serial("TLS: handshake complete", 50, m))
    check("page fetched (200)", d.wait_serial("BROWSER: GET https://henryratterman.com", 50, m))
    check("style.css fetched", d.wait_serial("style.css", 50, m))
    check("page rendered", d.wait_serial("BROWSER: rendered https://henryratterman.com", 60, m))
    time.sleep(1.0); d.move(1000, 700); d.dump("m34_henryratterman")
    print("--- serial (GET/render/decoded lines) ---")
    for line in d.serial()[m:].splitlines():
        if any(k in line for k in ("BROWSER: GET", "rendered https", "decoded", "is not a PNG", "items,")):
            print("  " + line)
    d.quit(); finish()
main()
