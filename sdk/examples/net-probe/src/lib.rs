//! A network-probe app: each frame it tries an HTTP GET and reports whether the
//! OS allowed it. Used to demonstrate capability-based security — without the
//! `network` permission the host refuses the call and the probe shows DENIED.

#![no_std]

use veil_sdk as v;
use veil_sdk::color;

#[no_mangle]
pub extern "C" fn render() {
    v::clear(color::BG);
    v::draw_text(20, 14, "Network Probe", color::ACCENT, 26);
    v::draw_text(20, 56, "Tries an HTTP GET each frame.", color::MUTED, 14);

    let mut buf = [0u8; 64];
    let n = v::http_get("/index.htm", &mut buf);
    if n > 0 {
        v::log("net=ok");
        v::draw_text(20, 96, "Network: ALLOWED", color::GREEN, 22);
    } else {
        v::log("net=denied");
        v::draw_text(20, 96, "Network: DENIED by OS", 0xffd0_5a4a, 22);
    }
}
