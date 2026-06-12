//! Boot animation: a 2-second animated Veil splash that plays before the
//! desktop (or setup screen) appears — the wordmark fades + scales in over a
//! dark gradient with a filling progress bar, then hands off smoothly to the
//! desktop. No raw text is shown to the visitor. Runs at full framebuffer
//! resolution and well within the 5-second visitor-boot target.

use crate::fb::Framebuffer;
use crate::freetype::FontId;

const BG_TOP: u32 = 0xff0a_0a14;
const BG_BOT: u32 = 0xff14_1830;
const ACCENT: u32 = 0xff5b_8af0;
const MUTED: u32 = 0xff70_7890;

/// Quadratic ease-out.
fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t) * (1.0 - t)
}

/// Animation parameters at normalized time `t` in [0, 1]:
/// (logo scale 0.6→1.0, logo alpha 0→255, progress 0.0→1.0).
pub fn frame_params(t: f32) -> (f32, u32, f32) {
    let t = t.clamp(0.0, 1.0);
    let scale = 0.6 + 0.4 * ease_out(t);
    // fade in over the first two-thirds, then hold opaque
    let alpha = ((t * 1.5).min(1.0) * 255.0) as u32;
    let progress = ease_out(t);
    (scale, alpha, progress)
}

/// Blend `fg` onto `bg` by `alpha` (0..255).
fn lerp_color(bg: u32, fg: u32, alpha: u32) -> u32 {
    let a = alpha.min(255);
    let mix = |s: u32, d: u32| -> u32 {
        let sb = (s >> 0) & 0xff;
        let db = (d >> 0) & 0xff;
        (db + ((sb as i32 - db as i32) * a as i32 / 255) as u32) & 0xff
    };
    let r = mix((fg >> 16) & 0xff, (bg >> 16) & 0xff);
    let g = mix((fg >> 8) & 0xff, (bg >> 8) & 0xff);
    let b = mix(fg & 0xff, bg & 0xff);
    0xff00_0000 | (r << 16) | (g << 8) | b
}

/// Draw a vertical gradient background (top→bottom) in `n` bands.
fn gradient_bg(fb: &Framebuffer) {
    let (w, h) = (fb.width, fb.height);
    let bands = 48usize;
    let bh = h.div_ceil(bands);
    for i in 0..bands {
        let t = i as u32 * 255 / bands as u32;
        let c = lerp_color(BG_TOP, BG_BOT, t);
        fb.fill_rect(0, i * bh, w, bh.min(h - (i * bh).min(h)), c);
    }
}

/// Render one boot-animation frame for normalized time `t`.
pub fn render_frame(fb: &Framebuffer, t: f32) {
    let (w, h) = (fb.width as isize, fb.height as isize);
    gradient_bg(fb);
    let (scale, alpha, progress) = frame_params(t);

    // Wordmark "veil" centered, scaled + faded in.
    let size = (44.0 * scale) as u16;
    let color = lerp_color(BG_BOT, 0xffe8_ecff, alpha);
    let tw = fb.measure_text("veil", FontId::Ui, size).0 as isize;
    let tx = ((w - tw) / 2).max(0) as usize;
    let ty = ((h / 2) - size as isize) .max(0) as usize;
    fb.draw_text(tx, ty, "veil", FontId::Ui, size, color);

    // A small accent underline pulses in beneath the wordmark.
    let ul_w = (tw as f32 * (0.4 + 0.6 * progress)) as usize;
    let ul_x = ((w - ul_w as isize) / 2).max(0) as usize;
    let ul_y = ty + size as usize + 8;
    if ul_w > 4 {
        fb.fill_round_rect(ul_x, ul_y, ul_w, 3, 1, lerp_color(BG_BOT, ACCENT, alpha));
    }

    // Tagline fades in after the logo.
    let tag_alpha = (((t - 0.4).max(0.0) / 0.6) * 255.0) as u32;
    let tag = "a from-scratch operating system";
    let tagw = fb.measure_text(tag, FontId::Ui, 15).0 as isize;
    fb.draw_text(((w - tagw) / 2).max(0) as usize, ul_y + 20, tag, FontId::Ui, 15, lerp_color(BG_BOT, MUTED, tag_alpha));

    // Progress bar near the bottom.
    let bar_w = (w / 3).max(120);
    let bar_x = ((w - bar_w) / 2).max(0) as usize;
    let bar_y = (h * 4 / 5) as usize;
    fb.fill_round_rect(bar_x, bar_y, bar_w as usize, 4, 2, 0xff20_2438);
    let fill = (bar_w as f32 * progress) as usize;
    if fill > 0 {
        fb.fill_round_rect(bar_x, bar_y, fill, 4, 2, ACCENT);
    }
}

/// Play the full boot animation into `screen` over ~`dur_ticks` (50 Hz ticks;
/// 100 = 2 s), presenting each frame. Returns when complete.
pub fn play(screen: &Framebuffer) {
    let dur: u64 = 90; // ~1.8 s at 50 Hz — comfortably inside the 5 s budget
    let start = crate::timer::ticks();
    loop {
        let elapsed = crate::timer::ticks().saturating_sub(start);
        let t = (elapsed as f32 / dur as f32).min(1.0);
        render_frame(screen, t);
        crate::gpu::present(); // no-op on the ramfb path
        if elapsed >= dur {
            break;
        }
        // brief spin so the tick counter advances (50 Hz timer drives `ticks`).
        for _ in 0..200_000 {
            core::hint::spin_loop();
        }
    }
}

/// Boot-animation self-test: verify the frame parameter curve.
pub fn selftest() {
    let (s0, a0, p0) = frame_params(0.0);
    let (s1, a1, p1) = frame_params(1.0);
    let (sm, am, pm) = frame_params(0.5);
    let start_ok = (s0 - 0.6).abs() < 0.01 && a0 == 0 && p0 < 0.01;
    let end_ok = (s1 - 1.0).abs() < 0.01 && a1 == 255 && (p1 - 1.0).abs() < 0.01;
    // midpoint strictly between, opaque by 2/3, ease-out progress (0.75 at t=0.5)
    let mid_ok = sm > s0 && sm < s1 && am > a0 && pm > p0 && pm < p1
        && (pm - 0.75).abs() < 0.02;
    // color blend endpoints
    let c0 = lerp_color(0xff00_0000, 0xffff_ffff, 0);
    let c255 = lerp_color(0xff00_0000, 0xffff_ffff, 255);
    let cmid = lerp_color(0xff00_0000, 0xffff_ffff, 128);
    let blend_ok = c0 == 0xff00_0000 && c255 == 0xffff_ffff
        && ((cmid & 0xff) as i32 - 128).abs() <= 2;
    crate::kprintln!("BOOTANIM: start={start_ok} end={end_ok} mid={mid_ok} blend={blend_ok} (mid: scale={sm:.2} alpha={am} prog={pm:.2})");
    if start_ok && end_ok && mid_ok && blend_ok {
        crate::kprintln!("BOOTANIM_OK: animated boot splash — wordmark fade+scale, gradient, progress bar; frame curve + color blend correct (plays ~1.8 s before the desktop)");
    } else {
        crate::kprintln!("BOOTANIM_FAIL: start={start_ok} end={end_ok} mid={mid_ok} blend={blend_ok}");
    }
}
