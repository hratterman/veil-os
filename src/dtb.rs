//! Minimal flattened-device-tree (DTB) parser — just enough to find a node
//! by `compatible` and read its properties. Spec §4: device addresses and
//! IRQ numbers must come from here, not be hard-coded.
//!
//! QEMU virt places the DTB at RAM base (0x4000_0000) on ELF `-kernel`
//! boots; x0 is only populated for Linux-format images.

const FDT_MAGIC: u32 = 0xd00d_feed;

const FDT_BEGIN_NODE: u32 = 1;
const FDT_END_NODE: u32 = 2;
const FDT_PROP: u32 = 3;
const FDT_NOP: u32 = 4;
const FDT_END: u32 = 9;

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Read `n` cells (32-bit big-endian each) starting at `off`, folded into
/// one u64. DTB addresses/sizes on virt are 2 cells.
pub fn cells(data: &[u8], off: usize, n: usize) -> u64 {
    let mut value = 0u64;
    for i in 0..n {
        value = (value << 32) | be32(&data[off + i * 4..]) as u64;
    }
    value
}

/// A node, identified by the offset of its FDT_BEGIN_NODE token.
#[derive(Clone, Copy, PartialEq)]
pub struct Node(usize);

pub struct Fdt<'a> {
    structure: &'a [u8],
    strings: &'a [u8],
    total_size: usize,
}

impl<'a> Fdt<'a> {
    /// # Safety
    /// `base` must point at a valid DTB that outlives `'a` and is never
    /// written while borrowed.
    pub unsafe fn new(base: *const u8) -> Option<Fdt<'a>> {
        let header = unsafe { core::slice::from_raw_parts(base, 40) };
        if be32(header) != FDT_MAGIC {
            return None;
        }
        let total_size = be32(&header[4..]) as usize;
        let struct_off = be32(&header[8..]) as usize;
        let strings_off = be32(&header[12..]) as usize;
        let struct_size = be32(&header[36..]) as usize;
        let strings_size = be32(&header[32..]) as usize;
        if struct_off + struct_size > total_size || strings_off + strings_size > total_size {
            return None;
        }
        let blob = unsafe { core::slice::from_raw_parts(base, total_size) };
        Some(Fdt {
            structure: &blob[struct_off..struct_off + struct_size],
            strings: &blob[strings_off..strings_off + strings_size],
            total_size,
        })
    }

    fn token(&self, off: usize) -> u32 {
        be32(&self.structure[off..])
    }

    fn prop_name(&self, name_off: usize) -> &'a [u8] {
        let rest = &self.strings[name_off..];
        let len = rest.iter().position(|&b| b == 0).unwrap_or(0);
        &rest[..len]
    }

    /// Step over the token at `off`. Returns (next_off, token).
    fn next(&self, mut off: usize) -> (usize, u32) {
        let tok = self.token(off);
        off += 4;
        match tok {
            FDT_BEGIN_NODE => {
                let name_len = self.structure[off..].iter().position(|&b| b == 0).unwrap() + 1;
                off = (off + name_len + 3) & !3;
            }
            FDT_PROP => {
                let len = be32(&self.structure[off..]) as usize;
                off = (off + 8 + len + 3) & !3;
            }
            _ => {}
        }
        (off, tok)
    }

    /// The root node (first FDT_BEGIN_NODE).
    pub fn root(&self) -> Node {
        let mut off = 0;
        loop {
            match self.token(off) {
                FDT_BEGIN_NODE => return Node(off),
                FDT_NOP => off += 4,
                _ => panic!("malformed DTB: no root node"),
            }
        }
    }

    /// Total size of the blob (header + all blocks) — used to reserve the
    /// DTB's physical footprint from the frame allocator.
    pub fn total_size(&self) -> usize {
        self.total_size
    }

    /// Find the first node with `device_type` equal to `value` (e.g. "memory").
    pub fn find_device_type(&self, value: &str) -> Option<Node> {
        self.find_by_prop("device_type", value)
    }

    fn find_by_prop(&self, prop: &str, value: &str) -> Option<Node> {
        self.find_by_prop_after(prop, value, None)
    }

    /// Like find_by_prop, but only nodes strictly after `after` — call in a
    /// loop to enumerate all matches (e.g. the 32 virtio-mmio slots).
    fn find_by_prop_after(&self, prop: &str, value: &str, after: Option<Node>) -> Option<Node> {
        let min_off = after.map_or(0, |n| n.0 + 1);
        let mut off = 0;
        let mut current = Node(0);
        loop {
            let tok_off = off;
            let (next_off, tok) = self.next(off);
            off = next_off;
            match tok {
                FDT_BEGIN_NODE => current = Node(tok_off),
                FDT_PROP => {
                    let len = be32(&self.structure[tok_off + 4..]) as usize;
                    let name_off = be32(&self.structure[tok_off + 8..]) as usize;
                    if current.0 >= min_off && self.prop_name(name_off) == prop.as_bytes() {
                        let data = &self.structure[tok_off + 12..tok_off + 12 + len];
                        if data.split(|&b| b == 0).any(|s| s == value.as_bytes()) {
                            return Some(current);
                        }
                    }
                }
                FDT_END_NODE | FDT_NOP => {}
                FDT_END => return None,
                _ => panic!("malformed DTB: bad token {tok}"),
            }
        }
    }

    /// Find the first node whose `compatible` string list contains `needle`.
    pub fn find_compatible(&self, needle: &str) -> Option<Node> {
        self.find_by_prop("compatible", needle)
    }

    /// Next `compatible` match after `after` (for enumerating duplicates).
    pub fn find_compatible_after(&self, needle: &str, after: Node) -> Option<Node> {
        self.find_by_prop_after("compatible", needle, Some(after))
    }

    /// Read a property of `node`. Properties precede child nodes, so stop at
    /// the first FDT_BEGIN_NODE after the node's own.
    pub fn prop(&self, node: Node, name: &str) -> Option<&'a [u8]> {
        let (mut off, tok) = self.next(node.0);
        debug_assert_eq!(tok, FDT_BEGIN_NODE);
        loop {
            let tok_off = off;
            let (next_off, tok) = self.next(off);
            off = next_off;
            match tok {
                FDT_PROP => {
                    let len = be32(&self.structure[tok_off + 4..]) as usize;
                    let name_off = be32(&self.structure[tok_off + 8..]) as usize;
                    if self.prop_name(name_off) == name.as_bytes() {
                        return Some(&self.structure[tok_off + 12..tok_off + 12 + len]);
                    }
                }
                FDT_NOP => {}
                _ => return None, // child node, node end, or blob end
            }
        }
    }

    /// (#address-cells, #size-cells) of the root — governs `reg` layout for
    /// its direct children (every device node we care about on virt).
    pub fn root_cells(&self) -> (usize, usize) {
        let root = self.root();
        let addr = self.prop(root, "#address-cells").map_or(2, |d| be32(d) as usize);
        let size = self.prop(root, "#size-cells").map_or(1, |d| be32(d) as usize);
        (addr, size)
    }
}
