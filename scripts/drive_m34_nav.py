#!/usr/bin/env python3
"""M34 follow-up: the CSS-engine improvements that make real sites (e.g.
henryratterman.com) render as a clean page instead of a duplicate-nav mess.

navtest.htm exercises, deterministically over loopback (no live network):
  - multi-class matching: `<nav class="navbar primary">` matched by `.navbar`
  - descendant selectors: `.navbar a { color: var(--link) }` colors the links
  - @media skip: a `@media (max-width:900px)` block that would hide the navbar
    and recolor its links red must be ignored (we render the desktop layout)
  - hidden overlay: `.overlay { opacity:0; pointer-events:none }` must NOT paint
  - scroll-reveal: `.reveal { opacity:0 }` (no pointer-events) MUST stay visible
  - rem units: `.navbar { gap: 2rem }` spaces the links apart
"""
import sys

from guilib import Driver, check, finish

CONTENT_X = 512
PAGE_Y = 54 + 20


def boxes(s, kind):
    import re
    return [(m[0], int(m[1]), int(m[2]), int(m[3]), int(m[4]))
            for m in re.findall(
                rf"BROWSER: {kind} '([^']+)' at \((-?\d+), (-?\d+)\) (\d+)x(\d+)", s)]


def click_link(d, href_sub):
    lk = [b for b in boxes(d.serial(), "link") if href_sub in b[0]]
    if not lk:
        return False
    _, x, y, w, h = lk[-1]
    d.click(CONTENT_X + x + w // 2, PAGE_Y + y + h // 2)
    return True


def main():
    d = Driver(sys.argv[1], sys.argv[2], sys.argv[3])
    m = len(d.serial())
    d.click(262, 768 - 20)
    check("browser launched", d.wait_serial("WM: launch 'browser'", 5, m))
    check("index rendered", d.wait_serial("BROWSER: rendered / -", 40))
    m = len(d.serial())
    click_link(d, "/web.htm")
    check("web page rendered", d.wait_serial("BROWSER: rendered /web.htm", 20, m))
    m = len(d.serial())
    check("navtest link", click_link(d, "/navtest.htm"))
    check("navtest rendered", d.wait_serial("BROWSER: rendered /navtest.htm", 20, m))

    d.move(1000, 700)
    img = d.dump("m34_nav")

    # All sampled content colours (skip the dark page bg and the navbar's own
    # #102a40 background band).
    def palette():
        seen = {}
        for y in range(PAGE_Y, PAGE_Y + 360, 2):
            for x in range(CONTENT_X + 22, CONTENT_X + 460, 3):
                seen[img.at(x, y)] = seen.get(img.at(x, y), 0) + 1
        return seen

    pal = palette()
    near = lambda c, t, tol=40: all(abs(a - b) <= tol for a, b in zip(c, t))

    # Descendant `.navbar a` color var(--link)=#f0a020 on the multi-class navbar.
    # Seeing it proves multi-class matching + descendant selectors AND that the
    # @media block (which would hide the navbar / recolor it red) was skipped.
    link_orange = [c for c in pal if near(c, (240, 160, 32))]
    check("multi-class + descendant nav links rendered (orange, @media skipped)",
          bool(link_orange), f"orange-ish hits: {link_orange[:4]}")
    red = [c for c in pal if near(c, (255, 0, 0), 30)]
    check("@media mobile override did NOT recolor links red", not red, f"red hits: {red[:4]}")

    # Hidden overlay (#ff00ff, opacity:0 + pointer-events:none) must be absent.
    magenta = [c for c in pal if c[0] > 180 and c[2] > 180 and c[1] < 90]
    check("opacity:0 + pointer-events:none overlay is hidden", not magenta,
          f"magenta hits: {magenta[:4]}")

    # Scroll-reveal text (#20e0a0, opacity:0 only) must stay visible.
    reveal = [c for c in pal if near(c, (32, 224, 160))]
    check("opacity:0 scroll-reveal text stays visible", bool(reveal),
          f"teal hits: {reveal[:4]}")

    # rem gap: the three nav links must be spaced apart (gap:2rem ~= 32px), not
    # touching. Check the second link starts well past the first link's end.
    nav = [b for b in boxes(d.serial(), "link") if b[0].endswith(("navhome.htm", "navwork.htm"))]
    nav = sorted(nav, key=lambda b: b[1])
    if len(nav) >= 2:
        (_, x0, _, w0, _), (_, x1, _, _, _) = nav[0], nav[1]
        check("rem gap spaces nav links apart", x1 - (x0 + w0) >= 16,
              f"gap = {x1 - (x0 + w0)}px")
    else:
        check("rem gap spaces nav links apart", False, "nav link boxes not found")

    d.quit()
    finish()


main()
