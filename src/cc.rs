//! M41 step 21: a from-scratch C-subset compiler that runs *inside* Veil and
//! emits a WASM module the on-OS WASM runtime executes — so you can write C in
//! the editor, `cc hello.c` in the shell, and run it, with no host machine.
//!
//! Supported C subset: `int` (and string literals as data pointers), function
//! definitions, local declarations, `=`, arithmetic (`+ - * / %`), comparisons,
//! `&& || !`, `if/else`, `while`, `for`, `return`, function calls, and the
//! built-ins `print("...")` and `print_int(expr)`. Everything is `i32`.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

// --- lexer --------------------------------------------------------------------

#[derive(Clone, PartialEq, Debug)]
enum Tok {
    Int(i64),
    Str(String),
    Ident(String),
    Punct(String),
    Eof,
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let b: Vec<char> = src.chars().collect();
    let mut i = 0;
    let n = b.len();
    let mut out = Vec::new();
    while i < n {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        // comments
        if c == '/' && i + 1 < n && b[i + 1] == '/' {
            while i < n && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && b[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == '#' {
            // skip preprocessor lines (e.g. #include) — we ignore them
            while i < n && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_digit() {
            let mut v = 0i64;
            while i < n && b[i].is_ascii_digit() {
                v = v * 10 + (b[i] as i64 - '0' as i64);
                i += 1;
            }
            out.push(Tok::Int(v));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == '_') {
                s.push(b[i]);
                i += 1;
            }
            out.push(Tok::Ident(s));
            continue;
        }
        if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < n && b[i] != '"' {
                if b[i] == '\\' && i + 1 < n {
                    let e = b[i + 1];
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        '\\' => '\\',
                        '"' => '"',
                        '0' => '\0',
                        other => other,
                    });
                    i += 2;
                } else {
                    s.push(b[i]);
                    i += 1;
                }
            }
            i += 1;
            out.push(Tok::Str(s));
            continue;
        }
        // multi-char punctuators
        let two: String = b[i..(i + 2).min(n)].iter().collect();
        if ["==", "!=", "<=", ">=", "&&", "||", "++", "--", "+=", "-="].contains(&two.as_str()) {
            out.push(Tok::Punct(two));
            i += 2;
            continue;
        }
        out.push(Tok::Punct(c.to_string()));
        i += 1;
    }
    out.push(Tok::Eof);
    Ok(out)
}

// --- AST ----------------------------------------------------------------------

enum Expr {
    Int(i64),
    Str(String),
    Var(String),
    Bin(String, alloc::boxed::Box<Expr>, alloc::boxed::Box<Expr>),
    Unary(String, alloc::boxed::Box<Expr>),
    Call(String, Vec<Expr>),
}

enum Stmt {
    Decl(String, Option<Expr>),
    Assign(String, Expr),
    ExprStmt(Expr),
    If(Expr, Vec<Stmt>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    For(alloc::boxed::Box<Stmt>, Expr, alloc::boxed::Box<Stmt>, Vec<Stmt>),
    Return(Option<Expr>),
}

struct Func {
    name: String,
    params: Vec<String>,
    body: Vec<Stmt>,
}

// --- parser -------------------------------------------------------------------

struct Parser {
    t: Vec<Tok>,
    p: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.t[self.p]
    }
    fn next(&mut self) -> Tok {
        let t = self.t[self.p].clone();
        self.p += 1;
        t
    }
    fn eat_punct(&mut self, s: &str) -> Result<(), String> {
        if self.peek() == &Tok::Punct(s.to_string()) {
            self.p += 1;
            Ok(())
        } else {
            Err(alloc::format!("expected '{s}', got {:?}", self.peek()))
        }
    }
    fn is_punct(&self, s: &str) -> bool {
        self.peek() == &Tok::Punct(s.to_string())
    }
    fn is_kw(&self, s: &str) -> bool {
        self.peek() == &Tok::Ident(s.to_string())
    }

    fn parse_program(&mut self) -> Result<Vec<Func>, String> {
        let mut funcs = Vec::new();
        while self.peek() != &Tok::Eof {
            funcs.push(self.parse_func()?);
        }
        Ok(funcs)
    }

    fn parse_func(&mut self) -> Result<Func, String> {
        // <type> name ( params ) { body }
        self.parse_type()?;
        let name = self.ident()?;
        self.eat_punct("(")?;
        let mut params = Vec::new();
        while !self.is_punct(")") {
            if self.is_kw("void") {
                self.next();
                break;
            }
            self.parse_type()?;
            params.push(self.ident()?);
            if self.is_punct(",") {
                self.next();
            }
        }
        self.eat_punct(")")?;
        let body = self.parse_block()?;
        Ok(Func { name, params, body })
    }

    fn parse_type(&mut self) -> Result<(), String> {
        // accept: int / char / void, plus any number of '*'
        match self.peek() {
            Tok::Ident(s) if ["int", "char", "void", "long", "unsigned"].contains(&s.as_str()) => {
                self.next();
            }
            other => return Err(alloc::format!("expected a type, got {other:?}")),
        }
        while self.is_punct("*") {
            self.next();
        }
        Ok(())
    }

    fn ident(&mut self) -> Result<String, String> {
        match self.next() {
            Tok::Ident(s) => Ok(s),
            other => Err(alloc::format!("expected identifier, got {other:?}")),
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.eat_punct("{")?;
        let mut stmts = Vec::new();
        while !self.is_punct("}") {
            stmts.push(self.parse_stmt()?);
        }
        self.eat_punct("}")?;
        Ok(stmts)
    }

    fn is_type(&self) -> bool {
        matches!(self.peek(), Tok::Ident(s) if ["int","char","void","long","unsigned"].contains(&s.as_str()))
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        if self.is_type() {
            // declaration: int x; / int x = e;
            self.parse_type()?;
            let name = self.ident()?;
            let init = if self.is_punct("=") {
                self.next();
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.eat_punct(";")?;
            return Ok(Stmt::Decl(name, init));
        }
        if self.is_kw("return") {
            self.next();
            let e = if self.is_punct(";") { None } else { Some(self.parse_expr()?) };
            self.eat_punct(";")?;
            return Ok(Stmt::Return(e));
        }
        if self.is_kw("if") {
            self.next();
            self.eat_punct("(")?;
            let cond = self.parse_expr()?;
            self.eat_punct(")")?;
            let then = self.parse_block_or_stmt()?;
            let els = if self.is_kw("else") {
                self.next();
                self.parse_block_or_stmt()?
            } else {
                Vec::new()
            };
            return Ok(Stmt::If(cond, then, els));
        }
        if self.is_kw("while") {
            self.next();
            self.eat_punct("(")?;
            let cond = self.parse_expr()?;
            self.eat_punct(")")?;
            let body = self.parse_block_or_stmt()?;
            return Ok(Stmt::While(cond, body));
        }
        if self.is_kw("for") {
            self.next();
            self.eat_punct("(")?;
            let init = self.parse_simple_stmt()?;
            self.eat_punct(";")?;
            let cond = self.parse_expr()?;
            self.eat_punct(";")?;
            let step = self.parse_simple_stmt()?;
            self.eat_punct(")")?;
            let body = self.parse_block_or_stmt()?;
            return Ok(Stmt::For(alloc::boxed::Box::new(init), cond, alloc::boxed::Box::new(step), body));
        }
        // assignment or expression statement
        let s = self.parse_simple_stmt()?;
        self.eat_punct(";")?;
        Ok(s)
    }

    /// A statement with no trailing ';' (used in `for` clauses).
    fn parse_simple_stmt(&mut self) -> Result<Stmt, String> {
        if self.is_type() {
            self.parse_type()?;
            let name = self.ident()?;
            let init = if self.is_punct("=") {
                self.next();
                Some(self.parse_expr()?)
            } else {
                None
            };
            return Ok(Stmt::Decl(name, init));
        }
        // lookahead: ident '=' / '+=' / '++'  -> assignment
        if let Tok::Ident(name) = self.peek().clone() {
            if self.t.get(self.p + 1) == Some(&Tok::Punct("=".to_string())) {
                self.next();
                self.next();
                let e = self.parse_expr()?;
                return Ok(Stmt::Assign(name, e));
            }
            if self.t.get(self.p + 1) == Some(&Tok::Punct("++".to_string())) {
                self.next();
                self.next();
                return Ok(Stmt::Assign(name.clone(), Expr::Bin("+".into(), Box::new(Expr::Var(name)), Box::new(Expr::Int(1)))));
            }
            if self.t.get(self.p + 1) == Some(&Tok::Punct("+=".to_string())) {
                self.next();
                self.next();
                let e = self.parse_expr()?;
                return Ok(Stmt::Assign(name.clone(), Expr::Bin("+".into(), Box::new(Expr::Var(name)), Box::new(e))));
            }
        }
        Ok(Stmt::ExprStmt(self.parse_expr()?))
    }

    fn parse_block_or_stmt(&mut self) -> Result<Vec<Stmt>, String> {
        if self.is_punct("{") {
            self.parse_block()
        } else {
            Ok(vec![self.parse_stmt()?])
        }
    }

    // Pratt-ish expression parser by precedence.
    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut l = self.parse_and()?;
        while self.is_punct("||") {
            self.next();
            let r = self.parse_and()?;
            l = Expr::Bin("||".into(), Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut l = self.parse_cmp()?;
        while self.is_punct("&&") {
            self.next();
            let r = self.parse_cmp()?;
            l = Expr::Bin("&&".into(), Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let mut l = self.parse_add()?;
        loop {
            let op = match self.peek() {
                Tok::Punct(p) if ["==", "!=", "<", ">", "<=", ">="].contains(&p.as_str()) => p.clone(),
                _ => break,
            };
            self.next();
            let r = self.parse_add()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut l = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Tok::Punct(p) if p == "+" || p == "-" => p.clone(),
                _ => break,
            };
            self.next();
            let r = self.parse_mul()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut l = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Punct(p) if p == "*" || p == "/" || p == "%" => p.clone(),
                _ => break,
            };
            self.next();
            let r = self.parse_unary()?;
            l = Expr::Bin(op, Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.is_punct("-") {
            self.next();
            return Ok(Expr::Unary("-".into(), Box::new(self.parse_unary()?)));
        }
        if self.is_punct("!") {
            self.next();
            return Ok(Expr::Unary("!".into(), Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Tok::Int(v) => Ok(Expr::Int(v)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Punct(p) if p == "(" => {
                let e = self.parse_expr()?;
                self.eat_punct(")")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                if self.is_punct("(") {
                    self.next();
                    let mut args = Vec::new();
                    while !self.is_punct(")") {
                        args.push(self.parse_expr()?);
                        if self.is_punct(",") {
                            self.next();
                        }
                    }
                    self.eat_punct(")")?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Var(name))
                }
            }
            other => Err(alloc::format!("unexpected token {other:?}")),
        }
    }
}

use alloc::boxed::Box;

// --- WASM emitter -------------------------------------------------------------

fn uleb(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
        }
        out.push(b);
        if v == 0 {
            break;
        }
    }
}
fn sleb(mut v: i64, out: &mut Vec<u8>) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        let sign = b & 0x40 != 0;
        if (v == 0 && !sign) || (v == -1 && sign) {
            out.push(b);
            break;
        }
        out.push(b | 0x80);
    }
}
fn i32c(v: i64, out: &mut Vec<u8>) {
    out.push(0x41);
    sleb(v, out);
}
fn section(id: u8, body: &[u8], out: &mut Vec<u8>) {
    out.push(id);
    let mut len = Vec::new();
    uleb(body.len() as u64, &mut len);
    out.extend_from_slice(&len);
    out.extend_from_slice(body);
}
fn name(s: &str, out: &mut Vec<u8>) {
    uleb(s.len() as u64, out);
    out.extend_from_slice(s.as_bytes());
}

struct Gen {
    funcs: BTreeMap<String, (usize, usize)>, // name -> (func index, nparams)
    data: Vec<u8>,
    strings: BTreeMap<String, (u32, u32)>, // literal -> (offset, len)
    data_off: u32,
}

const IMPORT_PRINT: usize = 0; // env.veil_log(i32 ptr, i32 len) -> ()
const IMPORT_PRINT_INT: usize = 1; // env.veil_print_int(i32) -> ()
const DATA_BASE: u32 = 1024;

/// Compile C source to a WASM module. Errors on the first problem.
pub fn compile(src: &str) -> Result<Vec<u8>, String> {
    let toks = lex(src)?;
    let mut p = Parser { t: toks, p: 0 };
    let prog = p.parse_program()?;
    if !prog.iter().any(|f| f.name == "main") {
        return Err("no main() function".to_string());
    }

    // Function index map: imports occupy 0,1; defined functions follow.
    let mut g = Gen {
        funcs: BTreeMap::new(),
        data: Vec::new(),
        strings: BTreeMap::new(),
        data_off: DATA_BASE,
    };
    for (i, f) in prog.iter().enumerate() {
        g.funcs.insert(f.name.clone(), (2 + i, f.params.len()));
    }

    // Code for each function.
    let mut codes: Vec<Vec<u8>> = Vec::new();
    for f in &prog {
        codes.push(gen_func(f, &mut g)?);
    }

    // --- assemble the module ---
    let mut module = Vec::new();
    module.extend_from_slice(b"\0asm");
    module.extend_from_slice(&[1, 0, 0, 0]);

    // type section: 0 = (i32,i32)->() [veil_log], 1 = (i32)->() [print_int],
    // then 2+k = (i32 * k) -> i32 for each function arity k.
    let max_np = prog.iter().map(|f| f.params.len()).max().unwrap_or(0);
    let mut types = Vec::new();
    uleb((2 + max_np + 1) as u64, &mut types);
    types.extend_from_slice(&[0x60, 2, 0x7f, 0x7f, 0]); // (i32,i32)->()
    types.extend_from_slice(&[0x60, 1, 0x7f, 0]); // (i32)->()
    for k in 0..=max_np {
        types.push(0x60);
        uleb(k as u64, &mut types);
        for _ in 0..k {
            types.push(0x7f);
        }
        uleb(1, &mut types); // one i32 result
        types.push(0x7f);
    }
    section(1, &types, &mut module);

    // import section: env.veil_log (type 0), env.veil_print_int (type 1)
    let mut imports = Vec::new();
    uleb(2, &mut imports);
    name("env", &mut imports);
    name("veil_log", &mut imports);
    imports.push(0x00);
    uleb(0, &mut imports);
    name("env", &mut imports);
    name("veil_print_int", &mut imports);
    imports.push(0x00);
    uleb(1, &mut imports);
    section(2, &imports, &mut module);

    // function section: function with k params uses type index 2+k.
    let mut funcs = Vec::new();
    uleb(prog.len() as u64, &mut funcs);
    for f in &prog {
        uleb((2 + f.params.len()) as u64, &mut funcs);
    }
    section(3, &funcs, &mut module);

    // memory: 2 pages (room for the string data + a small heap)
    let mut mem = Vec::new();
    uleb(1, &mut mem);
    mem.push(0x00);
    uleb(2, &mut mem);
    section(5, &mem, &mut module);

    // export section: every function by name, plus `_start` -> main, + memory
    let mut exports = Vec::new();
    let mut export_items: Vec<(String, usize)> = prog.iter().map(|f| (f.name.clone(), g.funcs[&f.name].0)).collect();
    let main_idx = g.funcs["main"].0;
    export_items.push(("_start".to_string(), main_idx));
    uleb((export_items.len() + 1) as u64, &mut exports);
    for (nm, idx) in &export_items {
        name(nm, &mut exports);
        exports.push(0x00);
        uleb(*idx as u64, &mut exports);
    }
    name("memory", &mut exports);
    exports.push(0x02);
    uleb(0, &mut exports);
    section(7, &exports, &mut module);

    // code section
    let mut code = Vec::new();
    uleb(codes.len() as u64, &mut code);
    for c in &codes {
        uleb(c.len() as u64, &mut code);
        code.extend_from_slice(c);
    }
    section(10, &code, &mut module);

    // data section (string literals)
    if !g.data.is_empty() {
        let mut data = Vec::new();
        uleb(1, &mut data);
        uleb(0, &mut data); // memory 0, active
        i32c(DATA_BASE as i64, &mut data);
        data.push(0x0b); // end
        uleb(g.data.len() as u64, &mut data);
        data.extend_from_slice(&g.data);
        section(11, &data, &mut module);
    }

    Ok(module)
}

fn gen_func(f: &Func, g: &mut Gen) -> Result<Vec<u8>, String> {
    // Collect all locals (params + declarations, hoisted), assign i32 slots.
    let mut locals: BTreeMap<String, usize> = BTreeMap::new();
    for (i, p) in f.params.iter().enumerate() {
        locals.insert(p.clone(), i);
    }
    collect_locals(&f.body, &mut locals);
    let n_extra = locals.len() - f.params.len();

    let mut body = Vec::new();
    for s in &f.body {
        gen_stmt(s, &locals, g, &mut body)?;
    }
    // Functions return i32; a default `return 0` covers fall-through (and is
    // dead code after an explicit return — valid WASM).
    i32c(0, &mut body);
    body.push(0x0b); // end

    // function header: local declarations (n_extra i32s)
    let mut out = Vec::new();
    if n_extra > 0 {
        uleb(1, &mut out);
        uleb(n_extra as u64, &mut out);
        out.push(0x7f); // i32
    } else {
        uleb(0, &mut out);
    }
    out.extend_from_slice(&body);
    Ok(out)
}

fn collect_locals(stmts: &[Stmt], locals: &mut BTreeMap<String, usize>) {
    for s in stmts {
        match s {
            Stmt::Decl(name, _) => {
                if !locals.contains_key(name) {
                    let idx = locals.len();
                    locals.insert(name.clone(), idx);
                }
            }
            Stmt::If(_, a, b) => {
                collect_locals(a, locals);
                collect_locals(b, locals);
            }
            Stmt::While(_, b) => collect_locals(b, locals),
            Stmt::For(init, _, step, b) => {
                collect_locals(core::slice::from_ref(init), locals);
                collect_locals(core::slice::from_ref(step), locals);
                collect_locals(b, locals);
            }
            _ => {}
        }
    }
}

fn gen_stmt(s: &Stmt, locals: &BTreeMap<String, usize>, g: &mut Gen, out: &mut Vec<u8>) -> Result<(), String> {
    match s {
        Stmt::Decl(name, init) => {
            if let Some(e) = init {
                gen_expr(e, locals, g, out)?;
                out.push(0x21); // local.set
                uleb(*locals.get(name).unwrap() as u64, out);
            }
        }
        Stmt::Assign(name, e) => {
            gen_expr(e, locals, g, out)?;
            let idx = *locals.get(name).ok_or_else(|| alloc::format!("unknown variable '{name}'"))?;
            out.push(0x21);
            uleb(idx as u64, out);
        }
        Stmt::ExprStmt(e) => {
            let pushed = gen_expr(e, locals, g, out)?;
            if pushed {
                out.push(0x1a); // drop the unused value
            }
        }
        Stmt::Return(e) => {
            match e {
                Some(e) => {
                    gen_expr(e, locals, g, out)?;
                }
                None => i32c(0, out),
            }
            out.push(0x0f); // return (leaves the i32)
        }
        Stmt::If(cond, then, els) => {
            gen_expr(cond, locals, g, out)?;
            out.push(0x04); // if
            out.push(0x40); // void
            for st in then {
                gen_stmt(st, locals, g, out)?;
            }
            if !els.is_empty() {
                out.push(0x05); // else
                for st in els {
                    gen_stmt(st, locals, g, out)?;
                }
            }
            out.push(0x0b); // end
        }
        Stmt::While(cond, body) => {
            out.push(0x02); // block
            out.push(0x40);
            out.push(0x03); // loop
            out.push(0x40);
            gen_expr(cond, locals, g, out)?;
            out.push(0x45); // i32.eqz
            out.push(0x0d); // br_if
            uleb(1, out); // exit the block
            for st in body {
                gen_stmt(st, locals, g, out)?;
            }
            out.push(0x0c); // br
            uleb(0, out); // back to loop
            out.push(0x0b); // end loop
            out.push(0x0b); // end block
        }
        Stmt::For(init, cond, step, body) => {
            gen_stmt(init, locals, g, out)?;
            out.push(0x02);
            out.push(0x40);
            out.push(0x03);
            out.push(0x40);
            gen_expr(cond, locals, g, out)?;
            out.push(0x45);
            out.push(0x0d);
            uleb(1, out);
            for st in body {
                gen_stmt(st, locals, g, out)?;
            }
            gen_stmt(step, locals, g, out)?;
            out.push(0x0c);
            uleb(0, out);
            out.push(0x0b);
            out.push(0x0b);
        }
    }
    Ok(())
}

/// Emit an expression; returns true if it leaves a value on the stack.
fn gen_expr(e: &Expr, locals: &BTreeMap<String, usize>, g: &mut Gen, out: &mut Vec<u8>) -> Result<bool, String> {
    match e {
        Expr::Int(v) => {
            i32c(*v, out);
            Ok(true)
        }
        Expr::Str(s) => {
            let (off, _len) = intern(g, s);
            i32c(off as i64, out);
            Ok(true)
        }
        Expr::Var(name) => {
            let idx = *locals.get(name).ok_or_else(|| alloc::format!("unknown variable '{name}'"))?;
            out.push(0x20); // local.get
            uleb(idx as u64, out);
            Ok(true)
        }
        Expr::Unary(op, e) => {
            if op == "-" {
                i32c(0, out);
                gen_expr(e, locals, g, out)?;
                out.push(0x6b); // i32.sub
            } else {
                gen_expr(e, locals, g, out)?;
                out.push(0x45); // i32.eqz  (logical not)
            }
            Ok(true)
        }
        Expr::Bin(op, a, b) => {
            gen_expr(a, locals, g, out)?;
            gen_expr(b, locals, g, out)?;
            let opcode = match op.as_str() {
                "+" => 0x6a,
                "-" => 0x6b,
                "*" => 0x6c,
                "/" => 0x6d,
                "%" => 0x6f,
                "==" => 0x46,
                "!=" => 0x47,
                "<" => 0x48,
                ">" => 0x4a,
                "<=" => 0x4c,
                ">=" => 0x4e,
                "&&" => 0x71, // i32.and (operands are 0/1-ish; fine for the subset)
                "||" => 0x72, // i32.or
                _ => return Err(alloc::format!("unsupported operator '{op}'")),
            };
            out.push(opcode);
            Ok(true)
        }
        Expr::Call(name, args) => {
            // built-ins
            if name == "print" {
                // print("literal") -> veil_log(ptr, len)
                if let [Expr::Str(s)] = args.as_slice() {
                    let (off, len) = intern(g, s);
                    i32c(off as i64, out);
                    i32c(len as i64, out);
                    out.push(0x10); // call
                    uleb(IMPORT_PRINT as u64, out);
                    return Ok(false);
                }
                return Err("print() takes a single string literal".to_string());
            }
            if name == "print_int" {
                if args.len() != 1 {
                    return Err("print_int() takes one int".to_string());
                }
                gen_expr(&args[0], locals, g, out)?;
                out.push(0x10);
                uleb(IMPORT_PRINT_INT as u64, out);
                return Ok(false);
            }
            // user function call
            let (idx, np) = *g.funcs.get(name).ok_or_else(|| alloc::format!("unknown function '{name}'"))?;
            if args.len() != np {
                return Err(alloc::format!("{name}() expects {np} args, got {}", args.len()));
            }
            for a in args {
                gen_expr(a, locals, g, out)?;
            }
            out.push(0x10);
            uleb(idx as u64, out);
            // user functions return i32.
            Ok(true)
        }
    }
}

/// Intern a string literal into the data segment; returns (offset, byte-len).
fn intern(g: &mut Gen, s: &str) -> (u32, u32) {
    if let Some(&(off, len)) = g.strings.get(s) {
        return (off, len);
    }
    let off = g.data_off;
    let bytes = s.as_bytes();
    g.data.extend_from_slice(bytes);
    g.data_off += bytes.len() as u32;
    g.strings.insert(s.to_string(), (off, bytes.len() as u32));
    (off, bytes.len() as u32)
}

/// Boot self-test: compile and run a small C program.
pub fn selftest() {
    let src = r#"
        int add(int a, int b) { return a + b; }
        int main() {
            print("Hello, Veil!");
            int sum = 0;
            for (int i = 1; i <= 10; i = i + 1) { sum = sum + i; }
            print_int(sum);
            print_int(add(20, 22));
            return 0;
        }
    "#;
    match compile(src) {
        Ok(wasm) => match crate::wasm::run(&wasm) {
            Ok(out) => {
                crate::kprintln!("CC: compiled {} bytes of WASM; output: {}", wasm.len(), out.trim().replace('\n', " | "));
                if out.contains("Hello, Veil!") && out.contains("55") && out.contains("42") {
                    crate::kprintln!("CC_OK: C-subset compiler built + ran a program inside Veil");
                } else {
                    crate::kprintln!("CC_FAIL: unexpected output {out:?}");
                }
            }
            Err(e) => crate::kprintln!("CC_FAIL: run error {e}"),
        },
        Err(e) => crate::kprintln!("CC_FAIL: compile error {e}"),
    }
}
