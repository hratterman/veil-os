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
    check("flextest link", click_link(d, "/flextest.htm"))
    check("flextest rendered", d.wait_serial("BROWSER: rendered /flextest.htm", 20, m))
    check("FLEX_OK", d.wait_serial("FLEX_OK", 6, m))
    # The four nav links must be on ONE row (same y) with increasing x — i.e.
    # laid out horizontally, not stacked. space-between should spread them.
    s = d.serial()[m:]
    nav = {}
    for href, x, y, w, h in boxes(s, "link"):
        for name in ("flexhome", "flexwork", "flexabout", "flexcontact"):
            if name in href:
                nav[name] = (x, y, w)
    check("all four nav links present", len(nav) == 4, str(nav))
    if len(nav) == 4:
        ys = [v[1] for v in nav.values()]
        xs_ordered = [nav[n][0] for n in ("flexhome", "flexwork", "flexabout", "flexcontact")]
        same_row = max(ys) - min(ys) <= 4
        increasing = all(xs_ordered[i] < xs_ordered[i + 1] for i in range(3))
        spread = xs_ordered[-1] - xs_ordered[0]
        check("nav links share one row (horizontal layout)", same_row, f"ys={ys}")
        check("nav links left-to-right", increasing, f"xs={xs_ordered}")
        check("space-between spreads them across the bar", spread > 260, f"spread={spread}")
    d.quit(); finish()
main()
