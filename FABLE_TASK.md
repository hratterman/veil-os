# Fable — M34: Browser Visual Overhaul

Read this in full before touching anything. Fully autonomous. Never ask for
approval. Stuck rule: 4 failed attempts → BLOCKED in PROGRESS.md, move on.
Build passes before every commit. Deploy after every task.

---

## Architecture reference

- Project root: `/Users/henry/projects/veil-os/`
- Browser: `src/browser.rs` — fetches via `http_get()`, lays out HTML, paints
- CSS parser: `src/css.rs`
- HTML parser: `src/html.rs`
- Font data: `src/font.rs` — current bitmap font (8x16 glyphs, ASCII)
- Framebuffer: `src/fb.rs` — pixel plotting primitives
- PNG decoder: `src/png.rs` (handles all color types, Adam7)
- TLS: `src/tls.rs` — working, used for https:// URLs
- No external Rust crates. No_std. AArch64.

---

## Task 1 — Fix HTTP response reading (Content-Length + chunked)

**This is a bug fix, do it first.** Sites like Wikipedia and neverssl.com
freeze the entire OS because `read_to_eof()` in `browser.rs` blocks forever
on keep-alive connections that never send EOF.

Fix `http_get()` to properly terminate the response read:

1. Parse `Content-Length` from the response headers. If present, read exactly
   that many bytes from the body then stop -- don't wait for EOF.
2. Parse `Transfer-Encoding: chunked`. If present, read chunk-encoded body:
   each chunk is `<hex-size>\r\n<data>\r\n`, terminated by `0\r\n\r\n`.
   Reassemble into the full body before returning.
3. If neither header is present, fall back to the existing EOF-wait with the
   existing timeout.
4. Also send `Connection: close` in the request headers for plain HTTP --
   this makes servers close after responding, avoiding the keep-alive hang
   on HTTP/1.1 servers.

This fix applies to both the proxy path (HTTP) and the TLS path (HTTPS).

Pass: serial emits `HTTP_READ_OK`. Test: fetch Wikipedia and neverssl.com --
both should load without freezing. Also verify example.com still works.

---

## Task 2 — External image fetching

The browser currently shows `[image]` for any `<img src="https://...">` or
`<img src="http://...">`. Fetch and display them.

In `browser.rs`, when rendering an `<img>` node:
- If `src` starts with `https://`, fetch via `tls_connect` (already works)
- If `src` starts with `http://`, fetch via the proxy (already works)  
- If `src` starts with `/` or is relative, fetch from loopback as now
- On success, decode with `png::decode()`. If the image is not PNG, skip it
  (show nothing -- most real sites use JPEG/WebP which we can't decode yet).
  Do not show `[image]` for non-PNG -- just leave the space empty.
- Cache decoded images in `BrowserState` by URL (a `Vec<(String, png::Image)>`)
  so navigating back doesn't re-fetch. Cap the cache at 10 images, evict LRU.
- Scale and display inline exactly as the existing PNG image rendering works.
- Fetch images concurrently if possible (spawn tasks via the scheduler), or
  sequentially if that's simpler -- sequential is fine, just don't block the
  whole render waiting for one slow image.

Pass: serial emits `EXT_IMG_OK` when an external PNG is successfully fetched
and rendered in the browser.

---

## Task 3 — CSS custom properties (variables)

Modern sites use `--variable-name: value` declarations and `var(--variable-name)`
references constantly. Currently silently ignored, which means most stylesheets
produce wrong colors/sizes.

In `src/css.rs`:
1. During stylesheet parsing, when a property name starts with `--`, store it
   in a `HashMap<String, String>` on the element's style (or a cascade-level
   variable map -- one per block scope).
2. When resolving any property value that contains `var(--foo)`, look up
   `--foo` in the current element's inherited variable map and substitute.
3. Variables inherit down the tree (defined on `:root` or `body` are available
   everywhere).
4. `var(--foo, fallback)` -- use fallback if `--foo` is not defined.

This doesn't need to be fully spec-compliant. Cover the 90% case: `:root`
level custom property declarations, `var()` substitution in color/font-size/
margin/padding/background-color values.

Pass: serial emits `CSS_VAR_OK`. Test: add a page to `mksite.py` that uses
CSS custom properties and verify it renders with the right colors.

---

## Task 4 — Flexbox layout

The single biggest layout gap. Sites use flexbox for navigation bars, card
grids, side-by-side columns -- everything. Without it, layouts collapse to
a single vertical stack.

In `src/browser.rs` layout engine, add flexbox support:

**Properties to implement:**
- `display: flex` on a container
- `flex-direction: row` (default) | `column`
- `flex-wrap: nowrap` (default) | `wrap`
- `justify-content: flex-start` (default) | `center` | `flex-end` |
  `space-between` | `space-around`
- `align-items: stretch` (default) | `center` | `flex-start` | `flex-end`
- `gap: <px>` (single value, applies to both axes)
- `flex: <number>` on children (grow factor, distribute remaining space)
- `flex-shrink` and `flex-basis` can be ignored for now

**Layout algorithm for `display: flex` containers:**
1. Measure all children's natural sizes (run normal block/inline layout on
   each child at unconstrained width to get their min-content width and height)
2. For `flex-direction: row`:
   - Distribute container width among children according to `flex` grow values
     (children with no `flex` get their natural width)
   - If `flex-wrap: wrap`, start a new row when children would overflow
   - Apply `justify-content` spacing along the main axis
   - Apply `align-items` on the cross axis
3. For `flex-direction: column`: same but vertically
4. Render each child into its allocated rect

This is the hardest task in the brief. Take your time. Test incrementally --
get `flex-direction: row` with `justify-content: space-between` working first
(that's the nav bar case), then add wrap and column.

Pass: serial emits `FLEX_OK`. Test against a page in `mksite.py` that uses
a flex nav bar and a flex card grid. Also verify henryratterman.com's nav
renders as a horizontal bar (test via proxy/TLS).

---

## Task 5 — Bitmap font expansion

The browser currently has one 8x16 bitmap font. Expand to support multiple
typefaces and weights, which will be selected via CSS `font-family`,
`font-weight`, and `font-style`.

### Fonts to pre-rasterize

Pre-rasterize these specific fonts at these sizes on the host using Python
(Pillow or freetype-py) and bake the bitmaps into the kernel as Rust arrays.
Write a script `scripts/gen_fonts.py` that:

1. Downloads (or uses system) the following Google Fonts:
   - **Cormorant Garamond** — weights 300, 400, 600; regular and italic
     (this is what henryratterman.com uses for headings)
   - **Lora** — weight 400; regular and italic (body text)
   - **Barlow Condensed** — weight 400, 600 (the all-caps nav/label style)
   - **JetBrains Mono** or **Fira Code** — weight 400 (monospace, for `<pre>`
     and the Lisp REPL)

2. Rasterizes each at **16px** and **24px** (two sizes covers most cases).
   Use freetype-py or Pillow's ImageFont with the TTF files.

3. Generates a Rust source file `src/fonts_generated.rs` containing:
   ```rust
   pub struct BitmapFont {
       pub glyph_w: usize,
       pub glyph_h: usize,
       pub data: &'static [u8], // 1 bit per pixel, row-major per glyph
   }
   pub static CORMORANT_400_16: BitmapFont = BitmapFont { ... };
   pub static CORMORANT_400_24: BitmapFont = BitmapFont { ... };
   pub static CORMORANT_600_16: BitmapFont = BitmapFont { ... };
   pub static CORMORANT_ITALIC_400_16: BitmapFont = BitmapFont { ... };
   pub static LORA_400_16: BitmapFont = BitmapFont { ... };
   pub static LORA_ITALIC_400_16: BitmapFont = BitmapFont { ... };
   pub static BARLOW_400_16: BitmapFont = BitmapFont { ... };
   pub static BARLOW_600_16: BitmapFont = BitmapFont { ... };
   pub static MONO_400_16: BitmapFont = BitmapFont { ... };
   // ... etc
   ```
   Cover printable ASCII (0x20–0x7E) for each variant. Glyphs don't have to
   be fixed-width (variable-width metrics are fine -- store advance widths).

4. In `src/font.rs`, add a font selection function:
   ```rust
   pub fn select_font(family: &str, weight: u16, italic: bool, size: u16) -> &'static BitmapFont
   ```
   Match `family` against "cormorant", "garamond", "lora", "barlow", "mono",
   "monospace", "serif", "sans-serif" etc. Fall back to the existing font if
   no match. Size: snap to nearest available (16 or 24).

5. In `browser.rs` layout/render, use `font::select_font()` with the computed
   CSS `font-family`/`font-weight`/`font-style` to pick the right bitmap font
   for each text run.

6. The Lisp REPL (`src/repl.rs`) should use the monospace font.

**Script requirements:**
- `scripts/gen_fonts.py` must be runnable standalone: `python3 scripts/gen_fonts.py`
- It downloads TTF files from Google Fonts API if not already cached in
  `assets/fonts/`
- It regenerates `src/fonts_generated.rs` -- this file is committed to the repo
- Add `python3 scripts/gen_fonts.py` as a step in `scripts/build.sh` so the
  fonts are always up to date

Pass: serial emits `FONTS_OK`. Visually verify in QEMU that a page using
`font-family: "Cormorant Garamond"` renders with the serif font, and that
`<pre>` in the browser uses the monospace font.

---

## Implementation order

1. Task 1 (HTTP read fix) -- bug fix, do first, unblocks everything
2. Task 2 (external images) -- high visual impact
3. Task 3 (CSS variables) -- enables modern stylesheets
4. Task 4 (flexbox) -- hardest, highest layout impact
5. Task 5 (fonts) -- visual polish, do last

After each task:
- `git add -A && git commit -m "M34: <task name>"`
- `scripts/install_sessions.sh`

---

## Target site for integration testing

After Tasks 1-4 are done, navigate to `https://henryratterman.com` in the
Veil browser (via TLS) and take a screenshot via the QEMU VNC framebuffer.
Compare against the real site. Note what renders correctly and what doesn't
in PROGRESS.md. This is the real-world acceptance test.

The site uses:
- Cormorant Garamond (headings, large serif)
- Barlow Condensed (nav, labels, all-caps)
- Lora (body italic)
- Flexbox for nav and card grid
- CSS custom properties for colors
- External images (headshots, project thumbnails) -- mostly JPEG, will show
  as empty space until JPEG support is added (that's fine for now)
- Dark green hero section (#1a2e1a or similar), cream body (#f5f0e8 or similar)

The goal: someone opens it in Veil and immediately recognizes it as a real
personal site, not a wall of text.

---

## Hard constraints

- No external Rust crates beyond what's already in Cargo.toml.
- `gen_fonts.py` may use Python packages (Pillow, freetype-py) -- install
  with pip if needed.
- Don't break existing functionality. Full regression suite before each commit.
- Never ask for confirmation. Make all decisions yourself.
- Don't touch TASK.md.
