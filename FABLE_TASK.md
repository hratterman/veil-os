# Fable Task — Internet Expansion + Video Player

Read this in full before touching a file.

---

## Context

Veil OS has a built-in browser (`src/browser.rs`) that fetches pages from the
OS's own HTTP server (`src/http.rs`) over loopback TCP. The HTTP server serves
files off the FAT16 disk. Pages are authored in `scripts/mksite.py`, which
writes HTML/CSS/PNG files into `site/`, which `scripts/mkdisk.sh` copies onto
the disk image verbatim. The browser supports: h1-h6, p, a, ul/ol/li, img
(PNG), div, span, br, pre, link (stylesheet). CSS: tag/.class selectors,
color, background-color, font-size, margin, padding, width, display. No JS,
no tables, no forms.

FAT 8.3 names are stored uppercase. Files served at `/foo.htm` must be named
`FOO.HTM` on disk. Max filename length is 12 chars (8.3 + dot).

The disk is 16MB. Be mindful of total size (HTML + CSS + PNG bytes all add up).

---

## Task 1 — Expand the browser internet (5+ new pages)

Currently there are two pages: `index.htm` and `page2.htm`. Expand this to
at least 7 total pages. They should feel like a real mini-internet that's fun
to explore. Ideas for what the pages could be (pick the best ones, add your
own ideas):

- A "news" page with fake headlines about Veil OS milestones as world events
- A Wikipedia-style article about something (the kernel, AArch64, memory paging)
- A "gallery" page that shows the generated PNG images in a grid
- An ASCII art page (pre tags, renders perfectly in the browser)
- A "terminal tips" page — keyboard shortcuts and commands
- A credits/about page
- An interactive changelog styled like a real product release page

Make the pages feel cohesive — consistent style, cross-linked navigation, good
writing. The style.css can be expanded (it's sparse right now: just bg, colors,
link color, pre style). Make it look good with the dark theme (#14181c bg).

All pages must use only the supported HTML/CSS subset described above. No JS,
no tables. Test every link — broken hrefs will 404 in the browser.

Implement in `scripts/mksite.py`. Add new entries to the `main()` function.
Update `index.htm` to link to new pages. Make sure all generated files are
written to the `site/` directory and are short enough (<=12 chars) for FAT 8.3.

---

## Task 2 — Animated GIF player app

Add a new OS app: `App::Gif`, an animated GIF player. The point is that a user
can upload any real `.gif` file via the pre-boot upload page (M30) and play it
inside the OS. This is the "I uploaded a GIF and it played in a from-scratch OS"
demo moment.

### Format: animated GIF

Implement a GIF89a decoder in `src/gif.rs`. Required features:
- GIF87a and GIF89a headers
- LZW decompression (the core codec, ~150 lines)
- Multiple frames (Graphic Control Extension: delay, disposal)
- Local and global color tables
- Interlaced frames (deinterlace on decode)
- Transparent color index support (composite onto background color)

You do NOT need to support: text extensions, plain text rendering, comments.
Unknown extension blocks must be skipped gracefully (read and discard).

The output of the decoder is `Vec<GifFrame>` where:
```rust
pub struct GifFrame {
    pub delay_cs: u16,   // centiseconds (from GCE), 0 = use 10cs default
    pub pixels: Vec<u32>, // ARGB, canvas_w * canvas_h, pre-composited
}
pub struct Gif {
    pub w: u16,
    pub h: u16,
    pub frames: Vec<GifFrame>,
}
```

### Sample GIF on the disk

Generate a sample animated GIF in pure Python (no PIL/numpy) in
`scripts/mksite.py` and write it to `site/demo.gif`. This ensures the app has
something to open even if no user uploads one. Keep it small: 64x64, ~12
frames, ~80ms per frame. Something visually interesting -- spinning color wheel,
plasma, bouncing ball with trail, etc.

GIF generation from scratch in Python: build the header, logical screen
descriptor, global color table, then per-frame image descriptors + LZW-
compressed image data. This is non-trivial but doable in ~150 lines of stdlib
Python. The LZW encoder for GIF uses a minimum code size based on color table
depth.

`mkdisk.sh` already copies everything in `site/` to the disk -- no changes
needed there.

Also update `mkdisk.sh` to accept `.gif` uploads alongside `.png` and `.wav`:
```bash
for f in "$SRC"/*.png "$SRC"/*.PNG "$SRC"/*.wav "$SRC"/*.WAV "$SRC"/*.gif "$SRC"/*.GIF; do
```
And update `site/landing.html` accept string and help text to include `.gif`.
And update `scripts/session_manager.py` similarly if it filters extensions.

### New Rust source file: `src/gif.rs`

Public API:
```rust
pub fn decode(data: &[u8]) -> Option<Gif>
```

### New Rust source file: `src/gifplayer.rs`

`GifPlayerState` struct, same pattern as `src/viewer.rs`:
```rust
pub struct GifPlayerState {
    files: Vec<String>,   // all .GIF names on disk, sorted
    idx: usize,           // which file is loaded
    gif: Option<Gif>,
    frame: usize,
    next_tick: u64,       // timer::ticks() when to advance frame
    playing: bool,
}
```

`GifPlayerState::new()` — list `.GIF` files, load first one, start playing.
`GifPlayerState::tick(win, input)` — space: play/pause, arrows: prev/next frame
or prev/next file (left/right = frames, up/down = files), Escape: close.
`GifPlayerState::render(win)` — blit current frame centered+scaled (nearest-
neighbor). Window title: `"DEMO.GIF [3/12] PLAYING"`.

### Wire into the desktop

In `src/desktop.rs`, add `App::Gif` with a desktop icon (colored box, label
"GIF" or a play symbol). Window size ~280x240. Wire into the main event loop
the same way `App::Viewer` is.

### Upload page

The M30 pre-boot upload page (`site/landing.html`) already supports `.png` and
`.wav`. Update it to also accept `.gif`. This means:
1. The `accept` attribute on the file input
2. The help text listing supported types
3. `session_manager.py` whitelisting `.gif`/`.GIF` for injection

### Pass criterion

Serial emits `GIF_OK` when the first frame of the first GIF on disk is decoded
and rendered. Add `kprintln!("GIF_OK")` in `GifPlayerState::new()` after
successful decode.

---

## Implementation order

1. Expand `mksite.py` with 5+ new pages + the GIF generator (`site/demo.gif`).
   Run it and check `site/` looks right.
2. Create `src/gif.rs` (LZW decoder + frame compositor).
3. Create `src/gifplayer.rs`.
4. Wire `App::Gif` into `src/desktop.rs` and the main event loop.
5. Update `mkdisk.sh`, `site/landing.html`, and `session_manager.py` to accept
   `.gif` uploads.
6. Build: `scripts/build.sh` — fix any compile errors.
7. `scripts/mkdisk.sh` — rebuild the disk.
8. Run QEMU. Confirm `GIF_OK` appears in serial output.
9. Manually verify: browser navigates all new pages; GIF app opens, plays
   `demo.gif`, space toggles play/pause.
10. `git add -A && git commit -m "M31: internet expansion + GIF player"`
11. Run `scripts/install_sessions.sh` to deploy to the hosted demo.

---

## Hard constraints

- No external crates beyond what's already in `Cargo.toml`.
- No tables or JS in the HTML — browser doesn't support them.
- All site filenames must be ≤12 chars (FAT 8.3 limit).
- Don't break anything that currently works: existing pages, viewer, audio,
  chat, paint, clock, editor. Run a build before committing.
- Don't change `TASK.md` — it's Fable's birth certificate.
- Don't delete or modify `os-build-spec-v2.md` or `os-build-spec-v3.md`.

---

## Where things live

- `/Users/henry/projects/veil-os/` — project root
- `src/` — kernel source (Rust)
- `scripts/mksite.py` — generates site HTML/CSS/PNG
- `scripts/mkdisk.sh` — builds the FAT16 disk image
- `scripts/install_sessions.sh` — deploys to hosted demo
- `site/` — generated output, copied to disk
- `assets/photos/` — real photos for the image viewer (don't touch)

