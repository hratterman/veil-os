//! WASM binary parser: the sections we need to run a module — types, imports,
//! functions, memory, globals, exports, code and data. No crates.

use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone)]
pub struct FuncType {
    pub params: Vec<u8>,
    pub results: Vec<u8>,
}

#[derive(Clone)]
pub struct Import {
    pub module: String,
    pub field: String,
    pub type_idx: u32,
}

#[derive(Clone)]
pub struct Func {
    pub type_idx: u32,
    pub locals: Vec<(u32, u8)>, // (count, valtype)
    pub body: Vec<u8>,          // raw bytecode, ending at the function `end`
}

#[derive(Clone)]
pub struct Export {
    pub name: String,
    pub kind: u8, // 0 func, 1 table, 2 mem, 3 global
    pub index: u32,
}

#[derive(Clone)]
pub struct DataSeg {
    pub offset: u32,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub struct Module {
    pub types: Vec<FuncType>,
    pub imports: Vec<Import>,
    pub func_types: Vec<u32>, // type index for each *defined* function
    pub funcs: Vec<Func>,
    pub exports: Vec<Export>,
    pub data: Vec<DataSeg>,
    pub globals: Vec<i64>,
    pub mem_min_pages: u32,
    pub num_imported_funcs: u32,
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.d.get(self.p)?;
        self.p += 1;
        Some(b)
    }

    /// Unsigned LEB128.
    fn uleb(&mut self) -> Option<u64> {
        let mut result = 0u64;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            result |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }

    /// Signed LEB128.
    fn sleb(&mut self) -> Option<i64> {
        let mut result = 0i64;
        let mut shift = 0;
        let mut b;
        loop {
            b = self.byte()?;
            result |= ((b & 0x7f) as i64) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
            if shift >= 64 {
                return None;
            }
        }
        if shift < 64 && (b & 0x40) != 0 {
            result |= -1i64 << shift;
        }
        Some(result)
    }

    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }

    fn name(&mut self) -> Option<String> {
        let n = self.uleb()? as usize;
        let b = self.bytes(n)?;
        Some(String::from_utf8_lossy(b).into_owned())
    }
}

pub fn parse(data: &[u8]) -> Option<Module> {
    if data.len() < 8 || &data[0..4] != b"\0asm" {
        return None;
    }
    let mut r = Reader { d: data, p: 8 };
    let mut m = Module { mem_min_pages: 0, ..Default::default() };

    while r.p < data.len() {
        let id = r.byte()?;
        let size = r.uleb()? as usize;
        let end = r.p + size;
        match id {
            1 => {
                // Type section.
                let n = r.uleb()?;
                for _ in 0..n {
                    let form = r.byte()?;
                    if form != 0x60 {
                        return None;
                    }
                    let np = r.uleb()? as usize;
                    let params = r.bytes(np)?.to_vec();
                    let nr = r.uleb()? as usize;
                    let results = r.bytes(nr)?.to_vec();
                    m.types.push(FuncType { params, results });
                }
            }
            2 => {
                // Import section.
                let n = r.uleb()?;
                for _ in 0..n {
                    let module = r.name()?;
                    let field = r.name()?;
                    let kind = r.byte()?;
                    match kind {
                        0 => {
                            let type_idx = r.uleb()? as u32;
                            m.imports.push(Import { module, field, type_idx });
                            m.num_imported_funcs += 1;
                        }
                        1 => {
                            r.byte()?; // elemtype
                            let flags = r.byte()?;
                            r.uleb()?;
                            if flags == 1 {
                                r.uleb()?;
                            }
                        }
                        2 => {
                            let flags = r.byte()?;
                            r.uleb()?;
                            if flags == 1 {
                                r.uleb()?;
                            }
                        }
                        3 => {
                            r.byte()?;
                            r.byte()?;
                        }
                        _ => return None,
                    }
                }
            }
            3 => {
                let n = r.uleb()?;
                for _ in 0..n {
                    m.func_types.push(r.uleb()? as u32);
                }
            }
            5 => {
                let n = r.uleb()?;
                for _ in 0..n {
                    let flags = r.byte()?;
                    let min = r.uleb()? as u32;
                    if flags == 1 {
                        r.uleb()?;
                    }
                    m.mem_min_pages = m.mem_min_pages.max(min);
                }
            }
            6 => {
                let n = r.uleb()?;
                for _ in 0..n {
                    let _vt = r.byte()?;
                    let _mut = r.byte()?;
                    // Constant init expr: i32.const N end (or i64).
                    let op = r.byte()?;
                    let v = match op {
                        0x41 => r.sleb()?,        // i32.const
                        0x42 => r.sleb()?,        // i64.const
                        _ => 0,
                    };
                    r.byte()?; // end (0x0b)
                    m.globals.push(v);
                }
            }
            7 => {
                let n = r.uleb()?;
                for _ in 0..n {
                    let name = r.name()?;
                    let kind = r.byte()?;
                    let index = r.uleb()? as u32;
                    m.exports.push(Export { name, kind, index });
                }
            }
            10 => {
                let n = r.uleb()?;
                for _ in 0..n {
                    let body_size = r.uleb()? as usize;
                    let body_end = r.p + body_size;
                    let nlocals = r.uleb()?;
                    let mut locals = Vec::new();
                    for _ in 0..nlocals {
                        let count = r.uleb()? as u32;
                        let vt = r.byte()?;
                        locals.push((count, vt));
                    }
                    let body = r.d.get(r.p..body_end)?.to_vec();
                    r.p = body_end;
                    let idx = m.funcs.len();
                    let type_idx = *m.func_types.get(idx)?;
                    m.funcs.push(Func { type_idx, locals, body });
                }
            }
            11 => {
                let n = r.uleb()?;
                for _ in 0..n {
                    let _mem_flags = r.uleb()?;
                    // offset const expr
                    let op = r.byte()?;
                    let offset = if op == 0x41 { r.sleb()? as u32 } else { 0 };
                    r.byte()?; // end
                    let len = r.uleb()? as usize;
                    let bytes = r.bytes(len)?.to_vec();
                    m.data.push(DataSeg { offset, bytes });
                }
            }
            _ => {} // skip custom / table / element / start sections
        }
        r.p = end;
    }
    Some(m)
}
