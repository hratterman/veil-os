//! VeilFS — a hierarchical filesystem (a real directory tree with absolute and
//! relative paths) layered on the virtio-blk device, replacing the flat FAT16
//! root for shell/editor use. The live tree is an arena (dirs hold child node
//! indices, files hold bytes); it is serialised to a reserved disk region
//! (LBA `VFS_LBA`) so the tree survives a reboot. On first boot it is seeded
//! with `/home/<user>` + dotfiles, `/bin`, `/usr/bin`, and `/tmp`.

use crate::blk;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::vec;

/// Disk region holding the serialised tree (high 2 MiB of the 16 MiB image,
/// in FAT16 free space). Sector 0 of the region holds an 8-byte magic + a u32
/// length, then the serialised bytes.
const VFS_LBA: u64 = 28672;
const VFS_SECTORS: usize = 4096; // 2 MiB cap on the persisted image
const MAGIC: &[u8; 8] = b"VEILFS01";

pub struct Node {
    pub name: String,
    pub is_dir: bool,
    pub children: Vec<usize>, // dir
    pub data: Vec<u8>,        // file
    pub mtime: u64,
    pub parent: usize,
}

pub struct Vfs {
    pub nodes: Vec<Node>,
    pub cwd: usize,
}

static mut VFS: Option<Vfs> = None;

pub fn get() -> &'static mut Vfs {
    unsafe {
        let p = core::ptr::addr_of_mut!(VFS);
        if (*p).is_none() {
            *p = Some(Vfs::fresh());
        }
        (*p).as_mut().unwrap()
    }
}

impl Vfs {
    fn empty() -> Vfs {
        let root = Node { name: String::from("/"), is_dir: true, children: Vec::new(), data: Vec::new(), mtime: now(), parent: 0 };
        Vfs { nodes: vec![root], cwd: 0 }
    }

    /// A fresh tree with the standard skeleton + a home dir and dotfiles.
    fn fresh() -> Vfs {
        let mut fs = Vfs::empty();
        let user = current_username();
        let home = alloc::format!("/home/{user}");
        for d in ["/home", &home, "/bin", "/usr", "/usr/bin", "/tmp", "/etc"] {
            fs.mkdir_p(d);
        }
        fs.write(&alloc::format!("{home}/.profile"), b"# ~/.profile - sourced on shell start\nexport PATH=/bin:/usr/bin\n");
        fs.write(&alloc::format!("{home}/.veilrc"), b"theme=dark\nwallpaper=sunset\n");
        fs.write(&alloc::format!("{home}/.history"), b"");
        fs.write(&alloc::format!("{home}/welcome.txt"), b"Welcome to Veil OS.\nThis is your home directory.\n");
        fs.write("/etc/hostname", b"veil\n");
        // start the session in the home directory
        fs.cwd = fs.resolve(&home).unwrap_or(0);
        fs
    }

    // ---- path resolution ---------------------------------------------------

    /// Resolve a path (absolute `/a/b`, or relative with `.`/`..`) to a node.
    pub fn resolve(&self, path: &str) -> Option<usize> {
        let mut cur = if path.starts_with('/') { 0 } else { self.cwd };
        for comp in path.split('/') {
            match comp {
                "" | "." => {}
                ".." => cur = self.nodes[cur].parent,
                name => {
                    cur = *self.nodes[cur].children.iter().find(|&&c| self.nodes[c].name == name)?;
                }
            }
        }
        Some(cur)
    }

    /// Split a path into (parent dir node, final component name).
    fn split_parent(&self, path: &str) -> Option<(usize, String)> {
        let path = path.trim_end_matches('/');
        let (dir, name) = match path.rfind('/') {
            Some(i) => (&path[..i + 1], &path[i + 1..]),
            None => ("", path),
        };
        if name.is_empty() {
            return None;
        }
        let parent = if dir.is_empty() { self.cwd } else { self.resolve(if dir == "/" { "/" } else { dir })? };
        Some((parent, name.to_string()))
    }

    fn child(&self, dir: usize, name: &str) -> Option<usize> {
        self.nodes[dir].children.iter().copied().find(|&c| self.nodes[c].name == name)
    }

    // ---- mutations ---------------------------------------------------------

    fn add(&mut self, parent: usize, name: &str, is_dir: bool) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(Node { name: name.to_string(), is_dir, children: Vec::new(), data: Vec::new(), mtime: now(), parent });
        self.nodes[parent].children.push(idx);
        idx
    }

    pub fn mkdir(&mut self, path: &str) -> Result<usize, &'static str> {
        let (parent, name) = self.split_parent(path).ok_or("bad path")?;
        if !self.nodes[parent].is_dir {
            return Err("not a directory");
        }
        if let Some(c) = self.child(parent, &name) {
            return if self.nodes[c].is_dir { Ok(c) } else { Err("file exists") };
        }
        Ok(self.add(parent, &name, true))
    }

    /// mkdir -p: create every missing component.
    pub fn mkdir_p(&mut self, path: &str) -> usize {
        let mut cur = if path.starts_with('/') { 0 } else { self.cwd };
        for comp in path.split('/') {
            match comp {
                "" | "." => {}
                ".." => cur = self.nodes[cur].parent,
                name => {
                    cur = match self.child(cur, name) {
                        Some(c) => c,
                        None => self.add(cur, name, true),
                    };
                }
            }
        }
        cur
    }

    pub fn write(&mut self, path: &str, data: &[u8]) -> Result<usize, &'static str> {
        let (parent, name) = self.split_parent(path).ok_or("bad path")?;
        if !self.nodes[parent].is_dir {
            return Err("not a directory");
        }
        let idx = match self.child(parent, &name) {
            Some(c) if !self.nodes[c].is_dir => c,
            Some(_) => return Err("is a directory"),
            None => self.add(parent, &name, false),
        };
        self.nodes[idx].data = data.to_vec();
        self.nodes[idx].mtime = now();
        Ok(idx)
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let idx = self.resolve(path)?;
        if self.nodes[idx].is_dir { None } else { Some(self.nodes[idx].data.clone()) }
    }

    pub fn ls(&self, path: &str) -> Option<Vec<(String, bool, usize)>> {
        let idx = self.resolve(path)?;
        if !self.nodes[idx].is_dir {
            let n = &self.nodes[idx];
            return Some(vec![(n.name.clone(), false, n.data.len())]);
        }
        let mut out: Vec<(String, bool, usize)> = self.nodes[idx]
            .children
            .iter()
            .map(|&c| (self.nodes[c].name.clone(), self.nodes[c].is_dir, self.nodes[c].data.len()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Some(out)
    }

    pub fn cd(&mut self, path: &str) -> Result<(), &'static str> {
        let idx = self.resolve(path).ok_or("no such directory")?;
        if !self.nodes[idx].is_dir {
            return Err("not a directory");
        }
        self.cwd = idx;
        Ok(())
    }

    pub fn remove(&mut self, path: &str) -> Result<(), &'static str> {
        let idx = self.resolve(path).ok_or("no such file")?;
        if idx == 0 {
            return Err("cannot remove /");
        }
        if self.nodes[idx].is_dir && !self.nodes[idx].children.is_empty() {
            return Err("directory not empty");
        }
        let parent = self.nodes[idx].parent;
        self.nodes[parent].children.retain(|&c| c != idx);
        // node left orphaned in the arena (compacted on next persist/load)
        Ok(())
    }

    /// Absolute path of a node, walking up to the root.
    pub fn path_of(&self, mut idx: usize) -> String {
        if idx == 0 {
            return String::from("/");
        }
        let mut parts: Vec<&str> = Vec::new();
        while idx != 0 {
            parts.push(&self.nodes[idx].name);
            idx = self.nodes[idx].parent;
        }
        parts.reverse();
        alloc::format!("/{}", parts.join("/"))
    }

    pub fn cwd_path(&self) -> String {
        self.path_of(self.cwd)
    }

    // ---- persistence -------------------------------------------------------

    /// Serialise the live tree (rooted at `/`) into a flat byte image.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.ser_node(0, &mut out);
        out
    }

    fn ser_node(&self, idx: usize, out: &mut Vec<u8>) {
        let n = &self.nodes[idx];
        put_str(out, &n.name);
        out.push(n.is_dir as u8);
        out.extend_from_slice(&n.mtime.to_le_bytes());
        if n.is_dir {
            out.extend_from_slice(&(n.children.len() as u32).to_le_bytes());
            for &c in &n.children {
                self.ser_node(c, out);
            }
        } else {
            out.extend_from_slice(&(n.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&n.data);
        }
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Vfs> {
        let mut fs = Vfs { nodes: Vec::new(), cwd: 0 };
        let mut p = 0usize;
        fs.de_node(bytes, &mut p, 0)?;
        if fs.nodes.is_empty() {
            return None;
        }
        Some(fs)
    }

    fn de_node(&mut self, b: &[u8], p: &mut usize, parent: usize) -> Option<usize> {
        let name = get_str(b, p)?;
        let is_dir = *b.get(*p)? != 0;
        *p += 1;
        let mtime = u64::from_le_bytes(b.get(*p..*p + 8)?.try_into().ok()?);
        *p += 8;
        let idx = self.nodes.len();
        self.nodes.push(Node { name, is_dir, children: Vec::new(), data: Vec::new(), mtime, parent });
        if is_dir {
            let n = u32::from_le_bytes(b.get(*p..*p + 4)?.try_into().ok()?) as usize;
            *p += 4;
            for _ in 0..n {
                let c = self.de_node(b, p, idx)?;
                self.nodes[idx].children.push(c);
            }
        } else {
            let len = u32::from_le_bytes(b.get(*p..*p + 4)?.try_into().ok()?) as usize;
            *p += 4;
            self.nodes[idx].data = b.get(*p..*p + len)?.to_vec();
            *p += len;
        }
        Some(idx)
    }

    /// Write the serialised tree to the reserved disk region.
    pub fn persist(&self) -> Result<(), ()> {
        let image = self.serialize();
        if image.len() + 12 > VFS_SECTORS * blk::SECTOR {
            return Err(()); // too large for the region
        }
        let mut buf = vec![0u8; VFS_SECTORS * blk::SECTOR];
        buf[..8].copy_from_slice(MAGIC);
        buf[8..12].copy_from_slice(&(image.len() as u32).to_le_bytes());
        buf[12..12 + image.len()].copy_from_slice(&image);
        let sectors = (12 + image.len()).div_ceil(blk::SECTOR);
        blk::write_sectors(VFS_LBA, sectors, &buf[..sectors * blk::SECTOR])
    }

    /// Load the tree from disk, or None if no valid image is stored.
    pub fn load() -> Option<Vfs> {
        let mut hdr = vec![0u8; blk::SECTOR];
        blk::read_sectors(VFS_LBA, 1, &mut hdr).ok()?;
        if &hdr[..8] != MAGIC {
            return None;
        }
        let len = u32::from_le_bytes(hdr[8..12].try_into().ok()?) as usize;
        if len == 0 || len > VFS_SECTORS * blk::SECTOR {
            return None;
        }
        let sectors = (12 + len).div_ceil(blk::SECTOR);
        let mut buf = vec![0u8; sectors * blk::SECTOR];
        blk::read_sectors(VFS_LBA, sectors, &mut buf).ok()?;
        let mut fs = Vfs::deserialize(&buf[12..12 + len])?;
        // resume in the user's home if it exists, else root
        let user = current_username();
        fs.cwd = fs.resolve(&alloc::format!("/home/{user}")).unwrap_or(0);
        Some(fs)
    }
}

fn now() -> u64 {
    crate::timer::wall_ticks50().map(|t| t / 50).unwrap_or(0)
}

/// The current user's name (USER.TXT), defaulting to "guest".
pub fn current_username() -> String {
    if let Some(data) = crate::fs::read_file("USER.TXT") {
        if let Ok(s) = core::str::from_utf8(&data) {
            let name: String = s.trim().chars().take(20).filter(|c| !c.is_whitespace()).collect();
            if !name.is_empty() {
                return name;
            }
        }
    }
    String::from("guest")
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}
fn get_str(b: &[u8], p: &mut usize) -> Option<String> {
    let len = u16::from_le_bytes(b.get(*p..*p + 2)?.try_into().ok()?) as usize;
    *p += 2;
    let s = core::str::from_utf8(b.get(*p..*p + len)?).ok()?.to_string();
    *p += len;
    Some(s)
}

/// Initialise the global VFS at boot: load from disk if a valid image exists,
/// otherwise seed a fresh tree and persist it.
pub fn init() {
    let fs = match Vfs::load() {
        Some(fs) => {
            crate::kprintln!("VFS: loaded {} nodes from disk", fs.nodes.len());
            fs
        }
        None => {
            let fs = Vfs::fresh();
            crate::kprintln!("VFS: first boot — seeded {} nodes (/home, dotfiles, /bin)", fs.nodes.len());
            let _ = fs.persist();
            fs
        }
    };
    unsafe { *core::ptr::addr_of_mut!(VFS) = Some(fs) };
}

/// Boot self-test (M42 step 6): hierarchical ops + an on-disk persistence
/// round-trip through the real block device.
pub fn selftest() {
    let fs = get();
    // mkdir + cd + write + read with relative paths
    let _ = fs.mkdir("/projects");
    let _ = fs.cd("/projects");
    let _ = fs.write("test.txt", b"hello");
    let read_rel = fs.read("test.txt").map(|d| String::from_utf8_lossy(&d).into_owned());
    let read_abs = fs.read("/projects/test.txt").map(|d| String::from_utf8_lossy(&d).into_owned());
    // .. and . navigation: /projects/sub -> .. -> /projects -> ./. stays
    let _ = fs.mkdir("/projects/sub");
    let _ = fs.cd("/projects/sub");
    let _ = fs.cd("..");
    let _ = fs.cd("./.");
    let cwd = fs.cwd_path();
    // home + dotfiles
    let user = current_username();
    let profile = fs.read(&alloc::format!("/home/{user}/.profile")).is_some();
    let home_dir = fs.resolve(&alloc::format!("/home/{user}")).map(|i| fs.nodes[i].is_dir).unwrap_or(false);
    // ls of /
    let root_entries = fs.ls("/").map(|v| v.len()).unwrap_or(0);

    // Persistence round-trip THROUGH THE DISK: persist, then load a fresh tree
    // from the block device and confirm the file is there.
    let persisted = fs.persist().is_ok();
    let reloaded = Vfs::load();
    let survives = reloaded
        .as_ref()
        .and_then(|f| f.read("/projects/test.txt"))
        .map(|d| d == b"hello")
        .unwrap_or(false);

    crate::kprintln!(
        "VFS: read_rel={read_rel:?} read_abs={read_abs:?} cwd={cwd} home={home_dir} profile={profile} root_entries={root_entries} persisted={persisted} survives={survives}"
    );
    let ok = read_rel.as_deref() == Some("hello")
        && read_abs.as_deref() == Some("hello")
        && cwd == "/projects"
        && home_dir
        && profile
        && root_entries >= 5
        && persisted
        && survives;
    if ok {
        crate::kprintln!("FS2_OK: hierarchical FS — mkdir/cd/relative paths, /home + dotfiles, and an on-disk persistence round-trip (survives reboot) all work");
    } else {
        crate::kprintln!("FS2_FAIL: read_rel={read_rel:?} cwd={cwd} home={home_dir} survives={survives}");
    }
}
