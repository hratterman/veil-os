//! FAT16 filesystem (root directory only, 8.3 names, read + write).
//! FAT16 so the same code carries to a real SD card on the Pi (M17), and
//! so the host can build/verify disk images with standard tools.
//!
//! All public entry points mask IRQs for their whole duration: the block
//! device has a single request slot and callers run in different tasks.

use crate::{blk, kprintln};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const SEC: usize = blk::SECTOR;
const ATTR_LFN: u8 = 0x0f;
const ATTR_VOLUME: u8 = 0x08;
const ATTR_DIR: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const EOC: u16 = 0xffff; // end-of-chain we write; >= 0xfff8 on read

#[derive(Clone, Copy)]
struct Layout {
    spc: usize,          // sectors per cluster
    fat_start: usize,    // sector of FAT #0
    fat_sectors: usize,  // per FAT
    num_fats: usize,
    root_start: usize,   // first root-directory sector
    root_sectors: usize,
    root_entries: usize,
    data_start: usize,   // first data sector (cluster 2)
}

static mut FS: Option<Layout> = None;

fn critical<R>(f: impl FnOnce() -> R) -> R {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack));
        core::arch::asm!("msr daifset, #2", options(nomem, nostack));
    }
    let result = f();
    unsafe { core::arch::asm!("msr daif, {}", in(reg) daif, options(nomem, nostack)) };
    result
}

fn layout() -> Option<Layout> {
    unsafe { *core::ptr::addr_of!(FS) }
}

fn le16(b: &[u8], off: usize) -> usize {
    u16::from_le_bytes([b[off], b[off + 1]]) as usize
}

/// Parse the BPB at sector 0. Returns false if there's no FAT16 here.
pub fn mount() -> bool {
    critical(|| {
        let mut bpb = [0u8; SEC];
        if blk::read_sectors(0, 1, &mut bpb).is_err() {
            return false;
        }
        if bpb[510] != 0x55 || bpb[511] != 0xaa || le16(&bpb, 11) != SEC {
            kprintln!("FS: sector 0 is not a 512-byte-sector FAT volume");
            return false;
        }
        let spc = bpb[13] as usize;
        let reserved = le16(&bpb, 14);
        let num_fats = bpb[16] as usize;
        let root_entries = le16(&bpb, 17);
        let fat_sectors = le16(&bpb, 22);
        if spc == 0 || num_fats == 0 || fat_sectors == 0 || root_entries == 0 {
            kprintln!("FS: BPB shape is not FAT16");
            return false;
        }
        let root_start = reserved + num_fats * fat_sectors;
        let root_sectors = (root_entries * 32).div_ceil(SEC);
        let layout = Layout {
            spc,
            fat_start: reserved,
            fat_sectors,
            num_fats,
            root_start,
            root_sectors,
            root_entries,
            data_start: root_start + root_sectors,
        };
        unsafe { *core::ptr::addr_of_mut!(FS) = Some(layout) };
        true
    })
}

/// "readme.txt" -> b"README  TXT". None if it doesn't fit 8.3.
fn to_83(name: &str) -> Option<[u8; 11]> {
    let mut out = [b' '; 11];
    let mut parts = name.rsplitn(2, '.');
    let (stem, ext) = match (parts.next(), parts.next()) {
        (Some(e), Some(s)) => (s, e),
        _ => (name, ""),
    };
    if stem.is_empty() || stem.len() > 8 || ext.len() > 3 {
        return None;
    }
    for (i, b) in stem.bytes().enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().enumerate() {
        out[8 + i] = b.to_ascii_uppercase();
    }
    Some(out)
}

fn from_83(raw: &[u8]) -> String {
    let stem = core::str::from_utf8(&raw[..8]).unwrap_or("").trim_end();
    let ext = core::str::from_utf8(&raw[8..11]).unwrap_or("").trim_end();
    let mut s = String::from(stem);
    if !ext.is_empty() {
        s.push('.');
        s.push_str(ext);
    }
    s
}

fn read_fat(l: &Layout, cluster: u16) -> u16 {
    let byte = cluster as usize * 2;
    let mut sec = [0u8; SEC];
    let _ = blk::read_sectors((l.fat_start + byte / SEC) as u64, 1, &mut sec);
    u16::from_le_bytes([sec[byte % SEC], sec[byte % SEC + 1]])
}

fn write_fat(l: &Layout, cluster: u16, value: u16) {
    let byte = cluster as usize * 2;
    for fat in 0..l.num_fats {
        let lba = (l.fat_start + fat * l.fat_sectors + byte / SEC) as u64;
        let mut sec = [0u8; SEC];
        let _ = blk::read_sectors(lba, 1, &mut sec);
        sec[byte % SEC..byte % SEC + 2].copy_from_slice(&value.to_le_bytes());
        let _ = blk::write_sectors(lba, 1, &sec);
    }
}

fn cluster_lba(l: &Layout, cluster: u16) -> u64 {
    (l.data_start + (cluster as usize - 2) * l.spc) as u64
}

/// (name, size, first_cluster, root dir slot) for every real root entry.
fn scan_root(l: &Layout) -> Vec<(String, u32, u16, usize)> {
    let mut out = Vec::new();
    let mut sec = [0u8; SEC];
    for s in 0..l.root_sectors {
        if blk::read_sectors((l.root_start + s) as u64, 1, &mut sec).is_err() {
            break;
        }
        for e in 0..SEC / 32 {
            let entry = &sec[e * 32..e * 32 + 32];
            let slot = s * (SEC / 32) + e;
            match entry[0] {
                0x00 => return out, // end of directory
                0xe5 => continue,   // deleted
                _ => {}
            }
            let attr = entry[11];
            if attr & ATTR_LFN == ATTR_LFN || attr & (ATTR_VOLUME | ATTR_DIR) != 0 {
                continue;
            }
            out.push((
                from_83(&entry[..11]),
                u32::from_le_bytes(entry[28..32].try_into().unwrap()),
                u16::from_le_bytes(entry[26..28].try_into().unwrap()),
                slot,
            ));
        }
    }
    out
}

pub fn list_root() -> Option<Vec<(String, u32)>> {
    critical(|| {
        let l = layout()?;
        Some(scan_root(&l).into_iter().map(|(n, s, _, _)| (n, s)).collect())
    })
}

pub fn read_file(name: &str) -> Option<Vec<u8>> {
    critical(|| {
        let l = layout()?;
        let name83 = to_83(name)?;
        let want = from_83(&name83);
        let (_, size, mut cluster, _) =
            scan_root(&l).into_iter().find(|(n, _, _, _)| *n == want)?;
        let cluster_bytes = l.spc * SEC;
        let mut data = vec![0u8; (size as usize).div_ceil(cluster_bytes) * cluster_bytes];
        let mut off = 0;
        while cluster >= 2 && cluster < 0xfff8 && off < data.len() {
            blk::read_sectors(cluster_lba(&l, cluster), l.spc, &mut data[off..off + cluster_bytes])
                .ok()?;
            off += cluster_bytes;
            cluster = read_fat(&l, cluster);
        }
        data.truncate(size as usize);
        Some(data)
    })
}

/// Create or overwrite `name` in the root directory.
pub fn write_file(name: &str, data: &[u8]) -> Result<(), ()> {
    critical(|| {
        let l = layout().ok_or(())?;
        let name83 = to_83(name).ok_or(())?;
        let want = from_83(&name83);
        let existing = scan_root(&l).into_iter().find(|(n, _, _, _)| *n == want);

        // Free any old chain.
        if let Some((_, _, mut cluster, _)) = existing {
            while cluster >= 2 && cluster < 0xfff8 {
                let next = read_fat(&l, cluster);
                write_fat(&l, cluster, 0);
                cluster = next;
            }
        }

        // Allocate a fresh chain.
        let cluster_bytes = l.spc * SEC;
        let needed = data.len().div_ceil(cluster_bytes).max(1);
        let total_clusters = l.fat_sectors * SEC / 2;
        let mut chain = Vec::with_capacity(needed);
        let mut candidate: u16 = 2;
        while chain.len() < needed && (candidate as usize) < total_clusters {
            if read_fat(&l, candidate) == 0 {
                chain.push(candidate);
            }
            candidate += 1;
        }
        if chain.len() < needed {
            kprintln!("FS: disk full");
            return Err(());
        }
        for i in 0..chain.len() {
            let next = if i + 1 < chain.len() { chain[i + 1] } else { EOC };
            write_fat(&l, chain[i], next);
        }

        // Write the data, zero-padding the final cluster.
        let mut buf = vec![0u8; cluster_bytes];
        for (i, &cluster) in chain.iter().enumerate() {
            let off = i * cluster_bytes;
            let n = data.len().saturating_sub(off).min(cluster_bytes);
            buf[..n].copy_from_slice(&data[off..off + n]);
            buf[n..].fill(0);
            blk::write_sectors(cluster_lba(&l, cluster), l.spc, &buf).map_err(|_| ())?;
        }

        // Directory entry: reuse the file's old slot, else first free one.
        let slot = match existing {
            Some((_, _, _, slot)) => slot,
            None => find_free_slot(&l).ok_or(())?,
        };
        let mut sec = [0u8; SEC];
        let lba = (l.root_start + slot / (SEC / 32)) as u64;
        blk::read_sectors(lba, 1, &mut sec).map_err(|_| ())?;
        let e = (slot % (SEC / 32)) * 32;
        sec[e..e + 32].fill(0);
        sec[e..e + 11].copy_from_slice(&name83);
        sec[e + 11] = ATTR_ARCHIVE;
        sec[e + 26..e + 28].copy_from_slice(&chain[0].to_le_bytes());
        sec[e + 28..e + 32].copy_from_slice(&(data.len() as u32).to_le_bytes());
        blk::write_sectors(lba, 1, &sec).map_err(|_| ())
    })
}

fn find_free_slot(l: &Layout) -> Option<usize> {
    let mut sec = [0u8; SEC];
    for s in 0..l.root_sectors {
        blk::read_sectors((l.root_start + s) as u64, 1, &mut sec).ok()?;
        for e in 0..SEC / 32 {
            if matches!(sec[e * 32], 0x00 | 0xe5) {
                return Some(s * (SEC / 32) + e);
            }
        }
    }
    None
}

pub fn mounted() -> bool {
    layout().is_some()
}
