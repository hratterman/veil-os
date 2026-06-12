//! A from-scratch WebGL 1.0 context: a small GLSL interpreter (lexer → parser →
//! tree-walking evaluator) and a software triangle rasteriser that runs the
//! vertex shader per vertex and the fragment shader per covered pixel,
//! interpolating varyings barycentrically. Output is composited into the owning
//! `<canvas>`'s pixel buffer so it shows where the canvas sits in layout.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::boxed::Box;

// ---- GLSL values ------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum Glsl {
    F(f32),
    V2([f32; 2]),
    V3([f32; 3]),
    V4([f32; 4]),
    M4([f32; 16]),
}

impl Glsl {
    fn comps(&self) -> &[f32] {
        match self {
            Glsl::F(_) => core::slice::from_ref(match self { Glsl::F(x) => x, _ => unreachable!() }),
            Glsl::V2(v) => v,
            Glsl::V3(v) => v,
            Glsl::V4(v) => v,
            Glsl::M4(v) => v,
        }
    }
    pub fn vec(comps: &[f32]) -> Glsl {
        match comps.len() {
            0 => Glsl::F(0.0),
            1 => Glsl::F(comps[0]),
            2 => Glsl::V2([comps[0], comps[1]]),
            3 => Glsl::V3([comps[0], comps[1], comps[2]]),
            _ => Glsl::V4([comps[0], comps[1], comps[2], comps[3]]),
        }
    }
}

// ---- GLSL AST ---------------------------------------------------------------

#[derive(Clone)]
enum Expr {
    Num(f32),
    Ident(String),
    Call(String, Vec<Expr>),
    Member(Box<Expr>, String),   // a.xyz / a.rgb swizzle
    Bin(char, Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
}

#[derive(Clone)]
enum Stmt {
    // declaration with optional initializer (qualifier handled separately)
    Local(String, Option<Expr>),
    Assign(Expr, Expr),
    Return,
}

struct Shader {
    attributes: Vec<String>,
    uniforms: Vec<String>,
    varyings: Vec<String>,
    body: Vec<Stmt>,
}

// ---- GLSL lexer/parser ------------------------------------------------------

#[derive(Clone, PartialEq)]
enum Tok { Id(String), Num(f32), P(char), Eof }

fn lex(src: &str) -> Vec<Tok> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        let c = b[i];
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' { i += 1; }
            continue;
        }
        if c.is_ascii_whitespace() { i += 1; continue; }
        if c == b'_' || c.is_ascii_alphabetic() {
            let s = i;
            while i < b.len() && (b[i] == b'_' || b[i].is_ascii_alphanumeric()) { i += 1; }
            out.push(Tok::Id(String::from(&src[s..i])));
            continue;
        }
        if c.is_ascii_digit() || (c == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit()) {
            let s = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.' || b[i] == b'e'
                || b[i] == b'E' || ((b[i] == b'+' || b[i] == b'-') && i > s && (b[i-1] == b'e' || b[i-1] == b'E'))) {
                i += 1;
            }
            out.push(Tok::Num(src[s..i].parse::<f32>().unwrap_or(0.0)));
            continue;
        }
        out.push(Tok::P(c as char));
        i += 1;
    }
    out.push(Tok::Eof);
    out
}

struct Parser { t: Vec<Tok>, p: usize }
impl Parser {
    fn peek(&self) -> &Tok { self.t.get(self.p).unwrap_or(&Tok::Eof) }
    fn bump(&mut self) -> Tok { let t = self.t.get(self.p).cloned().unwrap_or(Tok::Eof); self.p += 1; t }
    fn is_p(&self, c: char) -> bool { matches!(self.peek(), Tok::P(x) if *x == c) }
    fn eat_p(&mut self, c: char) -> bool { if self.is_p(c) { self.p += 1; true } else { false } }
    fn id(&self) -> Option<String> { if let Tok::Id(s) = self.peek() { Some(s.clone()) } else { None } }

    fn parse_shader(&mut self) -> Shader {
        let mut sh = Shader { attributes: Vec::new(), uniforms: Vec::new(), varyings: Vec::new(), body: Vec::new() };
        while !matches!(self.peek(), Tok::Eof) {
            // qualifier declarations: attribute/uniform/varying/precision/const
            if let Some(kw) = self.id() {
                match kw.as_str() {
                    "attribute" | "uniform" | "varying" | "const" => {
                        self.bump(); // qualifier
                        // optional precision qualifier
                        if matches!(self.id().as_deref(), Some("lowp")|Some("mediump")|Some("highp")) { self.bump(); }
                        self.bump(); // type
                        let name = self.id().unwrap_or_default(); self.bump();
                        match kw.as_str() {
                            "attribute" => sh.attributes.push(name),
                            "uniform" => sh.uniforms.push(name),
                            "varying" => sh.varyings.push(name),
                            _ => {}
                        }
                        while !self.eat_p(';') && !matches!(self.peek(), Tok::Eof) { self.bump(); }
                        continue;
                    }
                    "precision" => { while !self.eat_p(';') && !matches!(self.peek(), Tok::Eof) { self.bump(); } continue; }
                    "void" => {
                        // void main() { ... }
                        self.bump();
                        let _ = self.id(); self.bump(); // main
                        self.eat_p('('); while !self.eat_p(')') && !matches!(self.peek(), Tok::Eof) { self.bump(); }
                        self.eat_p('{');
                        sh.body = self.parse_block();
                        continue;
                    }
                    _ => { self.bump(); }
                }
            } else { self.bump(); }
        }
        sh
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        let mut out = Vec::new();
        while !self.eat_p('}') && !matches!(self.peek(), Tok::Eof) {
            if let Some(s) = self.parse_stmt() { out.push(s); }
        }
        out
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        if self.is_p(';') { self.bump(); return None; }
        if let Some(kw) = self.id() {
            if kw == "return" { while !self.eat_p(';') && !matches!(self.peek(), Tok::Eof) { self.bump(); } return Some(Stmt::Return); }
            // local var decl: <type> name [= expr];   (types: float/vec2/vec3/vec4/mat4)
            if matches!(kw.as_str(), "float"|"vec2"|"vec3"|"vec4"|"mat4"|"int"|"bool") {
                self.bump(); // type
                if matches!(self.id().as_deref(), Some("lowp")|Some("mediump")|Some("highp")) { self.bump(); }
                let name = self.id().unwrap_or_default(); self.bump();
                let init = if self.eat_p('=') { Some(self.parse_expr()) } else { None };
                self.eat_p(';');
                return Some(Stmt::Local(name, init));
            }
        }
        // assignment: lvalue = expr;
        let lhs = self.parse_unary();
        if self.eat_p('=') {
            let rhs = self.parse_expr();
            self.eat_p(';');
            return Some(Stmt::Assign(lhs, rhs));
        }
        self.eat_p(';');
        None
    }

    fn parse_expr(&mut self) -> Expr { self.parse_add() }
    fn parse_add(&mut self) -> Expr {
        let mut e = self.parse_mul();
        while self.is_p('+') || self.is_p('-') {
            let op = if let Tok::P(c) = self.bump() { c } else { '+' };
            let r = self.parse_mul();
            e = Expr::Bin(op, Box::new(e), Box::new(r));
        }
        e
    }
    fn parse_mul(&mut self) -> Expr {
        let mut e = self.parse_unary();
        while self.is_p('*') || self.is_p('/') {
            let op = if let Tok::P(c) = self.bump() { c } else { '*' };
            let r = self.parse_unary();
            e = Expr::Bin(op, Box::new(e), Box::new(r));
        }
        e
    }
    fn parse_unary(&mut self) -> Expr {
        if self.eat_p('-') { return Expr::Neg(Box::new(self.parse_unary())); }
        self.parse_postfix()
    }
    fn parse_postfix(&mut self) -> Expr {
        let mut e = self.parse_primary();
        loop {
            if self.eat_p('.') {
                let m = self.id().unwrap_or_default(); self.bump();
                e = Expr::Member(Box::new(e), m);
            } else { break; }
        }
        e
    }
    fn parse_primary(&mut self) -> Expr {
        if self.eat_p('(') { let e = self.parse_expr(); self.eat_p(')'); return e; }
        match self.bump() {
            Tok::Num(n) => Expr::Num(n),
            Tok::Id(name) => {
                if self.eat_p('(') {
                    let mut args = Vec::new();
                    if !self.is_p(')') {
                        loop { args.push(self.parse_expr()); if !self.eat_p(',') { break; } }
                    }
                    self.eat_p(')');
                    Expr::Call(name, args)
                } else { Expr::Ident(name) }
            }
            _ => Expr::Num(0.0),
        }
    }
}

pub fn compile(src: &str) -> Shader {
    let mut p = Parser { t: lex(src), p: 0 };
    p.parse_shader()
}

// ---- GLSL evaluator ---------------------------------------------------------

struct Env<'a> {
    vars: BTreeMap<String, Glsl>,
    uniforms: &'a BTreeMap<String, Glsl>,
}

fn swizzle(v: &Glsl, s: &str) -> Glsl {
    let comps = v.comps();
    let idx = |c: u8| -> f32 {
        let n = match c { b'x'|b'r'|b's' => 0, b'y'|b'g'|b't' => 1, b'z'|b'b'|b'p' => 2, b'w'|b'a'|b'q' => 3, _ => 0 };
        *comps.get(n).unwrap_or(&0.0)
    };
    let out: Vec<f32> = s.bytes().map(idx).collect();
    Glsl::vec(&out)
}

fn flatten(args: &[Glsl]) -> Vec<f32> {
    let mut out = Vec::new();
    for a in args { out.extend_from_slice(a.comps()); }
    out
}

fn eval(e: &Expr, env: &Env) -> Glsl {
    match e {
        Expr::Num(n) => Glsl::F(*n),
        Expr::Ident(name) => env.vars.get(name).or_else(|| env.uniforms.get(name)).copied().unwrap_or(Glsl::F(0.0)),
        Expr::Neg(inner) => {
            let v = eval(inner, env);
            let c: Vec<f32> = v.comps().iter().map(|x| -x).collect();
            if let Glsl::M4(_) = v { Glsl::M4({ let mut m=[0.0;16]; for (i,x) in c.iter().enumerate(){m[i]=*x;} m }) } else { Glsl::vec(&c) }
        }
        Expr::Member(base, s) => swizzle(&eval(base, env), s),
        Expr::Bin(op, a, b) => binop(*op, eval(a, env), eval(b, env)),
        Expr::Call(name, args) => {
            let vals: Vec<Glsl> = args.iter().map(|a| eval(a, env)).collect();
            call(name, &vals)
        }
    }
}

fn binop(op: char, a: Glsl, b: Glsl) -> Glsl {
    // matrix * vector / matrix * matrix
    if let (Glsl::M4(m), op @ '*') = (&a, op) {
        let _ = op;
        match b {
            Glsl::V4(v) => return Glsl::V4(mat4_vec4(m, &v)),
            Glsl::M4(n) => return Glsl::M4(mat4_mul(m, &n)),
            _ => {}
        }
    }
    let (ca, cb) = (a.comps().to_vec(), b.comps().to_vec());
    let f = |x: f32, y: f32| match op { '+' => x + y, '-' => x - y, '*' => x * y, '/' => if y != 0.0 { x / y } else { 0.0 }, _ => 0.0 };
    let out: Vec<f32> = if ca.len() == cb.len() {
        ca.iter().zip(cb.iter()).map(|(x, y)| f(*x, *y)).collect()
    } else if cb.len() == 1 {
        ca.iter().map(|x| f(*x, cb[0])).collect()
    } else if ca.len() == 1 {
        cb.iter().map(|y| f(ca[0], *y)).collect()
    } else {
        ca.clone()
    };
    Glsl::vec(&out)
}

fn call(name: &str, args: &[Glsl]) -> Glsl {
    match name {
        "vec2" | "vec3" | "vec4" => {
            let want = match name { "vec2" => 2, "vec3" => 3, _ => 4 };
            let mut c = flatten(args);
            if c.len() == 1 { c = vec![c[0]; want]; } // vecN(scalar)
            c.resize(want, if c.is_empty() { 0.0 } else { 0.0 });
            Glsl::vec(&c[..want])
        }
        "float" => Glsl::F(args.first().map(|a| a.comps()[0]).unwrap_or(0.0)),
        "dot" => {
            let (x, y) = (args[0].comps(), args[1].comps());
            Glsl::F(x.iter().zip(y).map(|(a, b)| a * b).sum())
        }
        "length" => { let c = args[0].comps(); Glsl::F(fsqrt(c.iter().map(|x| x * x).sum::<f32>())) }
        "normalize" => {
            let c = args[0].comps();
            let len = fsqrt(c.iter().map(|x| x * x).sum::<f32>());
            let l = if len > 0.0 { len } else { 1.0 };
            Glsl::vec(&c.iter().map(|x| x / l).collect::<Vec<_>>())
        }
        "mix" => {
            let (x, y) = (args[0].comps(), args[1].comps());
            let t = args[2].comps()[0];
            Glsl::vec(&x.iter().zip(y).map(|(a, b)| a + (b - a) * t).collect::<Vec<_>>())
        }
        "max" => { let (x,y)=(args[0].comps(),args[1].comps()); Glsl::vec(&x.iter().zip(y.iter().cycle()).map(|(a,b)| a.max(*b)).collect::<Vec<_>>()) }
        "min" => { let (x,y)=(args[0].comps(),args[1].comps()); Glsl::vec(&x.iter().zip(y.iter().cycle()).map(|(a,b)| a.min(*b)).collect::<Vec<_>>()) }
        "abs" => Glsl::vec(&args[0].comps().iter().map(|x| x.abs()).collect::<Vec<_>>()),
        "sin" => Glsl::vec(&args[0].comps().iter().map(|x| super::mathf::sin(*x as f64) as f32).collect::<Vec<_>>()),
        "cos" => Glsl::vec(&args[0].comps().iter().map(|x| super::mathf::cos(*x as f64) as f32).collect::<Vec<_>>()),
        _ => *args.first().unwrap_or(&Glsl::F(0.0)),
    }
}

fn mat4_vec4(m: &[f32; 16], v: &[f32; 4]) -> [f32; 4] {
    // column-major (WebGL convention)
    let mut o = [0.0f32; 4];
    for r in 0..4 {
        o[r] = m[r] * v[0] + m[4 + r] * v[1] + m[8 + r] * v[2] + m[12 + r] * v[3];
    }
    o
}
fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0.0f32; 16];
    for c in 0..4 { for r in 0..4 {
        let mut s = 0.0;
        for k in 0..4 { s += a[k * 4 + r] * b[c * 4 + k]; }
        o[c * 4 + r] = s;
    }}
    o
}

/// Run a shader's statements, returning the resulting variable environment.
fn run_shader(sh: &Shader, mut vars: BTreeMap<String, Glsl>, uniforms: &BTreeMap<String, Glsl>) -> BTreeMap<String, Glsl> {
    let mut env = Env { vars: core::mem::take(&mut vars), uniforms };
    for st in &sh.body {
        match st {
            Stmt::Local(name, init) => {
                let v = init.as_ref().map(|e| eval(e, &env)).unwrap_or(Glsl::F(0.0));
                env.vars.insert(name.clone(), v);
            }
            Stmt::Assign(lhs, rhs) => {
                let v = eval(rhs, &env);
                if let Expr::Ident(name) = lhs {
                    env.vars.insert(name.clone(), v);
                } else if let Expr::Member(base, s) = lhs {
                    // assign to swizzle target (e.g. gl_FragColor.rgb = ...)
                    if let Expr::Ident(name) = &**base {
                        let mut cur = env.vars.get(name).copied().unwrap_or(Glsl::V4([0.0;4])).comps().to_vec();
                        for (k, c) in s.bytes().enumerate() {
                            let n = match c { b'x'|b'r' => 0, b'y'|b'g' => 1, b'z'|b'b' => 2, b'w'|b'a' => 3, _ => 0 };
                            if n < cur.len() { cur[n] = *v.comps().get(k).unwrap_or(&0.0); }
                        }
                        env.vars.insert(name.clone(), Glsl::vec(&cur));
                    }
                }
            }
            Stmt::Return => break,
        }
    }
    env.vars
}

// ---- GL state machine + rasteriser -----------------------------------------

pub struct Buffer { pub data: Vec<f32> }
pub struct AttribPtr { pub buffer: usize, pub size: usize, pub stride: usize, pub offset: usize, pub enabled: bool }

pub struct GlContext {
    pub canvas: usize, // index into Interp.canvases (the framebuffer)
    pub w: usize,
    pub h: usize,
    pub clear: [f32; 4],
    pub buffers: Vec<Buffer>,
    pub bound_array: usize,
    pub vert_src: String,
    pub frag_src: String,
    pub attribs: BTreeMap<usize, AttribPtr>, // by location
    pub attrib_names: Vec<String>,           // location -> name (vertex shader attribute order)
    pub uniforms: BTreeMap<String, Glsl>,
    pub uniform_locs: Vec<String>,           // location -> name
}

impl GlContext {
    pub fn new(canvas: usize, w: usize, h: usize) -> GlContext {
        GlContext {
            canvas, w, h, clear: [0.0, 0.0, 0.0, 1.0],
            buffers: Vec::new(), bound_array: 0,
            vert_src: String::new(), frag_src: String::new(),
            attribs: BTreeMap::new(), attrib_names: Vec::new(),
            uniforms: BTreeMap::new(), uniform_locs: Vec::new(),
        }
    }

    pub fn attrib_location(&mut self, name: &str) -> usize {
        if let Some(i) = self.attrib_names.iter().position(|n| n == name) { return i; }
        self.attrib_names.push(String::from(name));
        self.attrib_names.len() - 1
    }
    pub fn uniform_location(&mut self, name: &str) -> usize {
        if let Some(i) = self.uniform_locs.iter().position(|n| n == name) { return i; }
        self.uniform_locs.push(String::from(name));
        self.uniform_locs.len() - 1
    }

    /// Draw `count` vertices as triangles into the framebuffer `px`.
    pub fn draw_arrays(&self, px: &mut [u32], count: usize) {
        let vsh = compile(&self.vert_src);
        let fsh = compile(&self.frag_src);
        let mut tri = 0;
        while tri + 3 <= count {
            // run the vertex shader for the 3 vertices
            let mut clip = [[0.0f32; 4]; 3];
            let mut varys: [BTreeMap<String, Glsl>; 3] = [BTreeMap::new(), BTreeMap::new(), BTreeMap::new()];
            for k in 0..3 {
                let vi = tri + k;
                let mut vars: BTreeMap<String, Glsl> = BTreeMap::new();
                for (loc, name) in self.attrib_names.iter().enumerate() {
                    if let Some(ap) = self.attribs.get(&loc) {
                        if !ap.enabled { continue; }
                        if let Some(buf) = self.buffers.get(ap.buffer) {
                            let base = ap.offset + vi * (if ap.stride > 0 { ap.stride } else { ap.size });
                            let mut c = [0.0f32; 4];
                            for j in 0..ap.size { c[j] = *buf.data.get(base + j).unwrap_or(&0.0); }
                            vars.insert(name.clone(), Glsl::vec(&c[..ap.size]));
                        }
                    }
                }
                let out = run_shader(&vsh, vars, &self.uniforms);
                if let Some(Glsl::V4(p)) = out.get("gl_Position") { clip[k] = *p; }
                else if let Some(g) = out.get("gl_Position") { let c = g.comps(); for j in 0..c.len().min(4) { clip[k][j] = c[j]; } }
                // keep declared varyings for interpolation
                for vn in &vsh.varyings { if let Some(v) = out.get(vn) { varys[k].insert(vn.clone(), *v); } }
            }
            // perspective divide + viewport transform to screen pixels
            let mut sc = [[0.0f32; 3]; 3]; // x,y,depth(unused)
            for k in 0..3 {
                let w = if clip[k][3] != 0.0 { clip[k][3] } else { 1.0 };
                let ndc = [clip[k][0] / w, clip[k][1] / w, clip[k][2] / w];
                sc[k][0] = (ndc[0] * 0.5 + 0.5) * self.w as f32;
                sc[k][1] = (1.0 - (ndc[1] * 0.5 + 0.5)) * self.h as f32; // flip Y
                sc[k][2] = ndc[2];
            }
            self.raster_tri(px, &sc, &varys, &fsh);
            tri += 3;
        }
    }

    fn raster_tri(&self, px: &mut [u32], sc: &[[f32; 3]; 3], varys: &[BTreeMap<String, Glsl>; 3], fsh: &Shader) {
        let (x0, y0) = (sc[0][0], sc[0][1]);
        let (x1, y1) = (sc[1][0], sc[1][1]);
        let (x2, y2) = (sc[2][0], sc[2][1]);
        let min_x = ffloor(x0.min(x1).min(x2)).max(0.0) as i32;
        let max_x = fceil(x0.max(x1).max(x2)).min(self.w as f32 - 1.0) as i32;
        let min_y = ffloor(y0.min(y1).min(y2)).max(0.0) as i32;
        let max_y = fceil(y0.max(y1).max(y2)).min(self.h as f32 - 1.0) as i32;
        let area = edge(x0, y0, x1, y1, x2, y2);
        if area.abs() < 1e-6 { return; }
        for py in min_y..=max_y {
            for pxx in min_x..=max_x {
                let (fx, fy) = (pxx as f32 + 0.5, py as f32 + 0.5);
                let w0 = edge(x1, y1, x2, y2, fx, fy) / area;
                let w1 = edge(x2, y2, x0, y0, fx, fy) / area;
                let w2 = edge(x0, y0, x1, y1, fx, fy) / area;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
                // interpolate varyings barycentrically and run the fragment shader
                let mut vars: BTreeMap<String, Glsl> = BTreeMap::new();
                for vn in fsh.varyings.iter() {
                    let a = varys[0].get(vn).map(|g| g.comps().to_vec());
                    let b = varys[1].get(vn).map(|g| g.comps().to_vec());
                    let c = varys[2].get(vn).map(|g| g.comps().to_vec());
                    if let (Some(a), Some(b), Some(c)) = (a, b, c) {
                        let n = a.len();
                        let mut out = vec![0.0f32; n];
                        for j in 0..n { out[j] = a[j] * w0 + b[j] * w1 + c[j] * w2; }
                        vars.insert(vn.clone(), Glsl::vec(&out));
                    }
                }
                let env = run_shader(fsh, vars, &self.uniforms);
                let frag = env.get("gl_FragColor").copied().unwrap_or(Glsl::V4([1.0, 0.0, 1.0, 1.0]));
                let c = frag.comps();
                let argb = pack(c.get(3).copied().unwrap_or(1.0), c[0], *c.get(1).unwrap_or(&0.0), *c.get(2).unwrap_or(&0.0));
                let idx = py as usize * self.w + pxx as usize;
                if idx < px.len() { px[idx] = argb; }
            }
        }
    }

    pub fn clear(&self, px: &mut [u32]) {
        let argb = pack(self.clear[3], self.clear[0], self.clear[1], self.clear[2]);
        for p in px.iter_mut() { *p = argb; }
    }
}

fn edge(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

// no_std f32 helpers (core has no sqrt/floor/ceil on f32).
fn fsqrt(x: f32) -> f32 { super::mathf::sqrt(x as f64) as f32 }
fn ffloor(x: f32) -> f32 { super::mathf::floor(x as f64) as f32 }
fn fceil(x: f32) -> f32 { super::mathf::ceil(x as f64) as f32 }

fn pack(a: f32, r: f32, g: f32, b: f32) -> u32 {
    let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    (q(a) << 24) | (q(r) << 16) | (q(g) << 8) | q(b)
}
