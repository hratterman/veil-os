//! virtio-sound (device ID 25) over virtio-mmio — native PCM playback.
//!
//! M24's audio path. (The spec named Intel HDA, but that is a PCI-only
//! device and this kernel deliberately runs everything over virtio-mmio to
//! avoid PCIe; virtio-sound is the same idea over the transport we already
//! have.) Four virtqueues: control(0), event(1), tx(2), rx(3). We drive a
//! single output stream (id 0): SET_PARAMS → PREPARE → START, then feed
//! 16-bit-stereo PCM as a ring of period buffers on the tx queue, refilling
//! each as the device returns it, and STOP at the end.

use crate::{dtb, frames, fs, gic, kprintln, virtio};
use alloc::string::String;
use core::ptr::{copy_nonoverlapping, read_volatile, write_bytes, write_volatile};
use core::sync::atomic::{AtomicBool, Ordering};

const VIRTIO_ID_SOUND: u32 = 25;

// Control request codes (virtio-snd spec).
const R_PCM_SET_PARAMS: u32 = 0x0101;
const R_PCM_PREPARE: u32 = 0x0102;
const R_PCM_RELEASE: u32 = 0x0103;
const R_PCM_START: u32 = 0x0104;
const R_PCM_STOP: u32 = 0x0105;
const S_OK: u32 = 0x8000;

const FMT_S16: u8 = 5;
const RATE_44100: u8 = 6;

const STREAM_ID: u32 = 0; // first output stream
const NUM_BUFS: usize = 8; // tx period buffers in flight
const PERIOD: usize = 4096; // bytes per period (1024 stereo frames)

struct Snd {
    mmio: virtio::Mmio,
    ctrl: virtio::Queue,
    tx: virtio::Queue,
    ctrl_buf: usize, // req at +0, response at +256
    xfer: usize,     // NUM_BUFS * 4-byte xfer headers
    status: usize,   // NUM_BUFS * 8-byte status structs
    data: usize,     // NUM_BUFS * PERIOD PCM bytes
}

static mut SND: Option<Snd> = None;

fn snd() -> Option<&'static mut Snd> {
    unsafe { (*core::ptr::addr_of_mut!(SND)).as_mut() }
}

pub fn available() -> bool {
    unsafe { (*core::ptr::addr_of!(SND)).is_some() }
}

/// Probe the virtio-mmio slots for a sound device and bring up its queues.
pub fn init(fdt: &dtb::Fdt) -> bool {
    let (addr_cells, _) = fdt.root_cells();
    let mut node = fdt.find_compatible("virtio,mmio");
    while let Some(n) = node {
        let Some(reg) = fdt.prop(n, "reg") else { break };
        let base = dtb::cells(reg, 0, addr_cells) as usize;
        let mmio = virtio::Mmio { base };
        if mmio.probe() == Some(VIRTIO_ID_SOUND) {
            let irq = fdt.prop(n, "interrupts").expect("virtio-sound node without irq");
            assert!(dtb::cells(irq, 0, 1) == 0, "virtio-sound irq is not an SPI?");
            let intid = 32 + dtb::cells(irq, 4, 1) as u32;
            init_device(mmio, intid);
            return true;
        }
        node = fdt.find_compatible_after("virtio,mmio", n);
    }
    false
}

fn on_irq(_intid: u32) {
    // ACK the device-level interrupt so the used ring is visible.
    // The handler returning causes maybe_preempt() -> yield to the audio task.
    if let Some(dev) = snd() {
        dev.mmio.irq_ack();
    }
}

fn init_device(mmio: virtio::Mmio, intid: u32) {
    mmio.init(0).expect("virtio-sound feature negotiation failed");

    // Rings: control(0), event(1), tx(2), rx(3). We only drive control + tx,
    // but the device wants all of its queues set up before DRIVER_OK.
    let ctrl = virtio::Queue::new(8, frames::alloc_zeroed().expect("snd ctrl ring"));
    let event = virtio::Queue::new(8, frames::alloc_zeroed().expect("snd event ring"));
    let tx = virtio::Queue::new(64, frames::alloc_zeroed().expect("snd tx ring"));
    let rx = virtio::Queue::new(8, frames::alloc_zeroed().expect("snd rx ring"));
    assert!(virtio::Queue::bytes_needed(64) <= frames::FRAME_SIZE);
    mmio.setup_queue(0, &ctrl);
    mmio.setup_queue(1, &event);
    mmio.setup_queue(2, &tx);
    mmio.setup_queue(3, &rx);
    mmio.driver_ok();

    let ctrl_buf = frames::alloc_zeroed().expect("snd ctrl buf");
    let xfer = frames::alloc_zeroed().expect("snd xfer hdrs");
    let status = frames::alloc_zeroed().expect("snd status");
    let data = frames::alloc_contiguous(NUM_BUFS * PERIOD / frames::FRAME_SIZE)
        .expect("snd pcm buffers");

    kprintln!("SND: virtio-sound at {:#x}, INTID {intid}, output stream {STREAM_ID}", mmio.base);
    gic::register_handler(intid, on_irq);
    gic::set_edge(intid);
    gic::enable(intid);
    unsafe {
        *core::ptr::addr_of_mut!(SND) =
            Some(Snd { mmio, ctrl, tx, ctrl_buf, xfer, status, data });
    }
}

// --- GUI-triggered playback -------------------------------------------------
// The App::Audio play button drops a filename here and spawns `audio_task`
// as a kernel task, so the ~3-second blocking stream doesn't freeze the
// desktop event loop.

static mut PLAY_FILE: Option<String> = None;
static PLAYING: AtomicBool = AtomicBool::new(false);
static STOP_REQ: AtomicBool = AtomicBool::new(false);

/// True while a stream is actively playing (the App::Audio window polls
/// this to drive its elapsed display and button label).
pub fn is_playing() -> bool {
    PLAYING.load(Ordering::Relaxed)
}

/// Ask the playing stream to stop early (the Stop button).
pub fn stop() {
    STOP_REQ.store(true, Ordering::Relaxed);
}

/// Queue a WAV file for the audio task to play.
pub fn request(file: &str) {
    unsafe { *core::ptr::addr_of_mut!(PLAY_FILE) = Some(String::from(file)) };
}

/// Kernel-task entry: play whatever `request` last queued, once.
pub fn audio_task() {
    let file = unsafe { (*core::ptr::addr_of_mut!(PLAY_FILE)).take() };
    if let Some(f) = file {
        play_file(&f);
    }
}

/// Read a WAV off the disk, validate it's 16-bit stereo 44.1 kHz, play it,
/// and emit the AUDIO_OK sentinel on a clean run.
pub fn play_file(name: &str) {
    let Some(data) = fs::read_file(name) else {
        kprintln!("AUDIO: {name} not found");
        return;
    };
    let Some((rate, channels, bits, pcm)) = parse_wav(&data) else {
        kprintln!("AUDIO: {name} is not a PCM WAV");
        return;
    };
    kprintln!("AUDIO: {name} {rate} Hz {channels}ch {bits}-bit, {} bytes PCM", pcm.len());
    if rate != 44100 || channels != 2 || bits != 16 {
        kprintln!("AUDIO: unsupported format (need 44100/2/16)");
        return;
    }
    let played = play(pcm);
    kprintln!("AUDIO_OK: played {played} bytes of {name} ({} ms)", played as u64 * 1000 / (rate as u64 * channels as u64 * (bits as u64 / 8)));
}

/// Parse a RIFF/WAVE file, returning (sample_rate, channels, bits_per_sample,
/// PCM data slice). Only uncompressed PCM (format tag 1) is accepted.
pub fn parse_wav(data: &[u8]) -> Option<(u32, u16, u16, &[u8])> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return None;
    }
    let le16 = |o: usize| u16::from_le_bytes([data[o], data[o + 1]]);
    let le32 = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let (mut rate, mut channels, mut bits) = (0u32, 0u16, 0u16);
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = le32(pos + 4) as usize;
        let body = pos + 8;
        if id == b"fmt " && body + 16 <= data.len() {
            if le16(body) != 1 {
                return None; // not PCM
            }
            channels = le16(body + 2);
            rate = le32(body + 4);
            bits = le16(body + 14);
        } else if id == b"data" {
            let end = (body + len).min(data.len());
            return Some((rate, channels, bits, &data[body..end]));
        }
        pos = body + len + (len & 1); // chunks are word-aligned
    }
    None
}

/// Issue one control command (request bytes already framed) and return the
/// device's response code. Synchronous: poll the control used ring.
fn ctrl_cmd(dev: &mut Snd, req: &[u8]) -> u32 {
    unsafe {
        write_bytes(dev.ctrl_buf as *mut u8, 0, 512);
        copy_nonoverlapping(req.as_ptr(), dev.ctrl_buf as *mut u8, req.len());
    }
    dev.ctrl.write_desc(0, dev.ctrl_buf as u64, req.len() as u32, virtio::DESC_F_NEXT, 1);
    dev.ctrl.write_desc(1, (dev.ctrl_buf + 256) as u64, 4, virtio::DESC_F_WRITE, 0);
    dev.ctrl.push_avail(0);
    dev.mmio.notify(0);
    let mut spins = 0u64;
    while dev.ctrl.pop_used().is_none() {
        dev.mmio.irq_ack(); // an MMIO touch forces a TCG vCPU exit so QEMU's
        spins += 1; // device/audio timer can run and update the used ring
        assert!(spins < 2_000_000_000, "virtio-sound control hung");
    }
    unsafe { read_volatile((dev.ctrl_buf + 256) as *const u32) }
}

fn pcm_hdr(code: u32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&code.to_le_bytes());
    b[4..8].copy_from_slice(&STREAM_ID.to_le_bytes());
    b
}

/// Configure, start, stream `pcm` (16-bit stereo 44.1 kHz LE), and stop.
/// Returns the number of bytes played. Blocks until the device has
/// acknowledged every buffer (so it returns after real playback time).
pub fn play(pcm: &[u8]) -> usize {
    let Some(dev) = snd() else {
        kprintln!("SND: no device");
        return 0;
    };

    // SET_PARAMS: buffer_bytes, period_bytes, features=0, channels=2,
    // format=S16, rate=44100.
    let mut sp = [0u8; 24];
    sp[0..8].copy_from_slice(&pcm_hdr(R_PCM_SET_PARAMS));
    sp[8..12].copy_from_slice(&((NUM_BUFS * PERIOD) as u32).to_le_bytes());
    sp[12..16].copy_from_slice(&(PERIOD as u32).to_le_bytes());
    sp[16..20].copy_from_slice(&0u32.to_le_bytes()); // features
    sp[20] = 2; // channels
    sp[21] = FMT_S16;
    sp[22] = RATE_44100;
    let r = ctrl_cmd(dev, &sp);
    assert!(r == S_OK, "SND: SET_PARAMS failed {r:#x}");
    assert!(ctrl_cmd(dev, &pcm_hdr(R_PCM_PREPARE)) == S_OK, "SND: PREPARE failed");
    assert!(ctrl_cmd(dev, &pcm_hdr(R_PCM_START)) == S_OK, "SND: START failed");
    STOP_REQ.store(false, Ordering::Relaxed);
    PLAYING.store(true, Ordering::Relaxed);
    kprintln!("SND: stream started (44100 Hz, 16-bit stereo), {} bytes to play", pcm.len());

    // Capture buffer bases as plain addresses so the closures below don't
    // hold a borrow of `dev` (its tx queue is mutated as we stream).
    let (data_base, xfer_base, status_base) = (dev.data, dev.xfer, dev.status);
    // Fill period buffer `k` from offset `pos`; return chunk length.
    let load = |k: usize, pos: usize| -> usize {
        let chunk = (pcm.len() - pos).min(PERIOD);
        unsafe {
            copy_nonoverlapping(pcm.as_ptr().add(pos), (data_base + k * PERIOD) as *mut u8, chunk);
        }
        chunk
    };
    // Build + publish a tx descriptor chain for period buffer `k`.
    let submit = |tx: &mut virtio::Queue, k: usize, len: usize| {
        let xfer = xfer_base + k * 4;
        unsafe { write_volatile(xfer as *mut u32, STREAM_ID) };
        let h = (k * 3) as u16;
        tx.write_desc(h, xfer as u64, 4, virtio::DESC_F_NEXT, h + 1);
        tx.write_desc(h + 1, (data_base + k * PERIOD) as u64, len as u32, virtio::DESC_F_NEXT, h + 2);
        tx.write_desc(h + 2, (status_base + k * 8) as u64, 8, virtio::DESC_F_WRITE, 0);
        tx.push_avail(h);
    };

    let mut pos = 0usize;
    let mut inflight = 0usize;
    for k in 0..NUM_BUFS {
        let len = load(k, pos);
        if len == 0 {
            break;
        }
        submit(&mut dev.tx, k, len);
        pos += len;
        inflight += 1;
    }
    dev.mmio.notify(2);

    let mut spins = 0u64;
    while inflight > 0 {
        match dev.tx.pop_used() {
            Some(head) => {
                spins = 0;
                let k = head as usize / 3;
                inflight -= 1;
                if pos < pcm.len() && !STOP_REQ.load(Ordering::Relaxed) {
                    let len = load(k, pos);
                    submit(&mut dev.tx, k, len);
                    pos += len;
                    inflight += 1;
                    dev.mmio.notify(2);
                }
            }
            None => {
                // irq_ack forces a TCG vCPU exit so QEMU's audio timer can
                // run and mark buffers used. Then wfi suspends this task
                // until the next IRQ (which triggers preemption back to the
                // desktop). This gives the UI full scheduling slices while
                // audio streams without spinning.
                dev.mmio.irq_ack();
                unsafe { core::arch::asm!("wfi") };
            }
        }
    }

    ctrl_cmd(dev, &pcm_hdr(R_PCM_STOP));
    ctrl_cmd(dev, &pcm_hdr(R_PCM_RELEASE));
    PLAYING.store(false, Ordering::Relaxed);
    kprintln!("SND: stream complete, {} bytes played", pos);
    pos
}
