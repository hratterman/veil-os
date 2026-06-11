import sys, re, time
from guilib import Driver, check, finish, taskbar_xy
CONTENT_X = 512; PAGE_Y = 54 + 20
def boxes(s, kind):
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]
def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial()); d.click(*taskbar_xy(d, "browser"))
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    # Navigate to web.htm (the link page).
    m = len(d.serial())
    lk = [b for b in boxes(d.serial(), "link") if b[0] == "/web.htm"][-1]
    _, x, y, w, h = lk; d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))
    # Click the direct-TLS https://example.com link.
    m = len(d.serial())
    ext = [b for b in boxes(d.serial(), "link") if b[0] == "https://example.com"]
    check("https link present", len(ext) > 0, str([b[0] for b in boxes(d.serial(), 'link')][:10]))
    if not ext:
        finish(); return
    _, x, y, w, h = ext[-1]; d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    check("TLS_OK (handshake + 200 over direct TLS 1.3)", d.wait_serial("TLS_OK", 45, m))
    check("HTTPS_OK (https page rendered)", d.wait_serial("HTTPS_OK", 10, m))
    check("Example Domain content present",
          "BROWSER: rendered https page" in d.serial())
    d.quit(); finish()
main()
