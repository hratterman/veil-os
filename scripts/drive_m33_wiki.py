import sys, re, time
from guilib import Driver, check, finish
CONTENT_X = 512; PAGE_Y = 54 + 20
def boxes(s, kind):
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]
def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial()); d.click(262, 768 - 20)
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    m = len(d.serial())
    lk = [b for b in boxes(d.serial(), "link") if b[0] == "/web.htm"][-1]
    _, x, y, w, h = lk; d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))
    # Wikipedia over direct TLS — a chunked (Transfer-Encoding) response, the
    # case that used to hang the reader forever (no EOF on keep-alive).
    m = len(d.serial())
    ext = [b for b in boxes(d.serial(), "link") if "en.wikipedia.org" in b[0]]
    check("wikipedia link present", len(ext) > 0)
    if not ext:
        finish(); return
    _, x, y, w, h = ext[-1]; d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    # If the chunked reader hung, this GET line never appears (test fails fast,
    # not a real OS hang since QEMU keeps running).
    check("wikipedia fetched (GET completed, no hang)",
          d.wait_serial("BROWSER: GET https://en.wikipedia.org", 50, m))
    check("wikipedia rendered", d.wait_serial("BROWSER: rendered https://en.wikipedia.org", 50, m))
    # The desktop is still alive afterward: open the clock and see it launch.
    m = len(d.serial()); d.click(70 + 1 * 78 + 36, 768 - 20)
    check("desktop still responsive after big fetch (clock launches)",
          d.wait_serial("WM: launch 'clock'", 6, m))
    d.quit(); finish()
main()
