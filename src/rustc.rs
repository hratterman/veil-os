//! A from-scratch **Rust-subset compiler** that runs *inside* Veil. Full
//! `rustc` compiled to WASM is far too large for the 16 MB heap, so this is a
//! Rust-subset front-end (lexer → recursive-descent parser → a lite type/borrow
//! check) that lowers to the C subset the on-OS `cc` compiler already turns into
//! WASM — so you can write Rust in the Veil editor, `rustc hello.rs` in the
//! shell, and run it, with no host machine.
//!
//! Supported: `fn name(p: i32, …) -> i32 { … }`, `let`/`let mut` bindings (with
//! the real Rust **immutability check** — assigning to a non-`mut` binding is a
//! compile error), `i32` arithmetic/comparisons/`&& || !`, `if/else` (no parens),
//! `while`, `for i in a..b`, `return`, **trailing-expression returns** (the last
//! expression in a block with no `;` is its value), function calls, and the
//! `println!("{}", x)` / `print!`/`println!("literal")` macros.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::format;

// ---- lexer -----------------------------------------------------------------

#[derive(Clone, PartialEq, Debug)]
enum Tok {
    Ident(String),
    Int(i64),
    Str(String),
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
        if c.is_ascii_whitespace() { i += 1; continue; }
        if c == '/' && i + 1 < n && b[i + 1] == '/' { while i < n && b[i] != '\n' { i += 1; } continue; }
        if c == '/' && i + 1 < n && b[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(b[i] == '*' && b[i + 1] == '/') { i += 1; }
            i += 2;
            continue;
        }
        if c.is_ascii_digit() {
            let mut v = 0i64;
            while i < n && (b[i].is_ascii_digit() || b[i] == '_') {
                if b[i] != '_' { v = v * 10 + (b[i] as i64 - '0' as i64); }
                i += 1;
            }
            out.push(Tok::Int(v));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < n && (b[i].is_ascii_alphanumeric() || b[i] == '_') { s.push(b[i]); i += 1; }
            out.push(Tok::Ident(s));
            continue;
        }
        if c == '"' {
            i += 1;
            let mut s = String::new();
            while i < n && b[i] != '"' {
                if b[i] == '\\' && i + 1 < n {
                    s.push(match b[i + 1] { 'n' => '\n', 't' => '\t', '\\' => '\\', '"' => '"', o => o });
                    i += 2;
                } else { s.push(b[i]); i += 1; }
            }
            i += 1;
            out.push(Tok::Str(s));
            continue;
        }
        // multi-char punctuators (incl. Rust's `..`, `->`, `::`)
        let three: String = b[i..(i + 3).min(n)].iter().collect();
        if three == "..=" { out.push(Tok::Punct(three)); i += 3; continue; }
        let two: String = b[i..(i + 2).min(n)].iter().collect();
        if ["==", "!=", "<=", ">=", "&&", "||", "->", "..", "::", "+=", "-="].contains(&two.as_str()) {
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

// ---- parser + transpiler (Rust subset -> C subset) -------------------------

struct Tp {
    t: Vec<Tok>,
    p: usize,
    /// mutable bindings in the current function (for the immutability check).
    muts: BTreeSet<String>,
    /// all declared bindings + params (so reassign-before-declare is an error).
    declared: BTreeSet<String>,
}

impl Tp {
    fn peek(&self) -> &Tok { self.t.get(self.p).unwrap_or(&Tok::Eof) }
    fn peek2(&self) -> &Tok { self.t.get(self.p + 1).unwrap_or(&Tok::Eof) }
    fn next(&mut self) -> Tok { let t = self.t.get(self.p).cloned().unwrap_or(Tok::Eof); self.p += 1; t }
    fn is_p(&self, s: &str) -> bool { self.peek() == &Tok::Punct(s.to_string()) }
    fn eat_p(&mut self, s: &str) -> Result<(), String> {
        if self.is_p(s) { self.p += 1; Ok(()) } else { Err(format!("expected '{s}', got {:?}", self.peek())) }
    }
    fn is_kw(&self, s: &str) -> bool { self.peek() == &Tok::Ident(s.to_string()) }
    fn ident(&mut self) -> Result<String, String> {
        match self.next() { Tok::Ident(s) => Ok(s), o => Err(format!("expected identifier, got {o:?}")) }
    }

    fn program(&mut self) -> Result<String, String> {
        let mut c = String::new();
        while self.peek() != &Tok::Eof {
            if self.is_kw("fn") {
                c.push_str(&self.func()?);
                c.push('\n');
            } else {
                return Err(format!("expected `fn`, got {:?}", self.peek()));
            }
        }
        Ok(c)
    }

    fn skip_type(&mut self) -> Result<(), String> {
        // i32 / i64 / u32 / usize / bool / & / mut — we only model integers.
        while self.is_p("&") || self.is_kw("mut") { self.next(); }
        let _ = self.ident()?; // base type name
        Ok(())
    }

    fn func(&mut self) -> Result<String, String> {
        self.next(); // fn
        let name = self.ident()?;
        self.eat_p("(")?;
        self.muts.clear();
        self.declared.clear();
        let mut params: Vec<String> = Vec::new();
        while !self.is_p(")") {
            let pmut = if self.is_kw("mut") { self.next(); true } else { false };
            let pname = self.ident()?;
            if pmut { self.muts.insert(pname.clone()); }
            self.declared.insert(pname.clone());
            self.eat_p(":")?;
            self.skip_type()?;
            params.push(format!("int {pname}"));
            if self.is_p(",") { self.next(); }
        }
        self.eat_p(")")?;
        if self.is_p("->") { self.next(); self.skip_type()?; }
        let body = self.block(true)?;
        let plist = if params.is_empty() { "void".to_string() } else { params.join(", ") };
        Ok(format!("int {name}({plist}) {body}"))
    }

    /// A `{ … }` block. `is_fn` => a trailing expression becomes `return`.
    fn block(&mut self, is_fn: bool) -> Result<String, String> {
        self.eat_p("{")?;
        let mut c = String::from("{\n");
        while !self.is_p("}") {
            // A trailing expression: an expression not followed by `;` right
            // before the closing `}` is the block's value (a Rust-ism).
            let start = self.p;
            if let Some(stmt) = self.try_stmt()? {
                c.push_str("    ");
                c.push_str(&stmt);
                c.push('\n');
            } else {
                // parse as an expression; if `;` follows it's an expr-stmt,
                // otherwise it's the trailing (return) expression.
                let e = self.expr()?;
                if self.is_p(";") {
                    self.next();
                    c.push_str(&format!("    {e};\n"));
                } else {
                    // must be the block end -> trailing value
                    if !self.is_p("}") {
                        let _ = start;
                        return Err(format!("expected `;` or `}}`, got {:?}", self.peek()));
                    }
                    if is_fn {
                        c.push_str(&format!("    return {e};\n"));
                    } else {
                        c.push_str(&format!("    {e};\n"));
                    }
                }
            }
        }
        self.eat_p("}")?;
        c.push('}');
        Ok(c)
    }

    /// Try to parse a statement that isn't a bare expression. Returns None if the
    /// next tokens are an expression (handled by the block).
    fn try_stmt(&mut self) -> Result<Option<String>, String> {
        if self.is_kw("let") {
            self.next();
            let m = if self.is_kw("mut") { self.next(); true } else { false };
            let name = self.ident()?;
            if m { self.muts.insert(name.clone()); }
            self.declared.insert(name.clone());
            if self.is_p(":") { self.next(); self.skip_type()?; }
            let init = if self.is_p("=") { self.next(); Some(self.expr()?) } else { None };
            self.eat_p(";")?;
            return Ok(Some(match init {
                Some(e) => format!("int {name} = {e};"),
                None => format!("int {name};"),
            }));
        }
        if self.is_kw("return") {
            self.next();
            let e = if self.is_p(";") { String::from("0") } else { self.expr()? };
            self.eat_p(";")?;
            return Ok(Some(format!("return {e};")));
        }
        if self.is_kw("if") {
            return Ok(Some(self.if_stmt()?));
        }
        if self.is_kw("while") {
            self.next();
            let cond = self.expr()?;
            let body = self.block(false)?;
            return Ok(Some(format!("while ({cond}) {body}")));
        }
        if self.is_kw("for") {
            // for NAME in A..B { body }   /   A..=B
            self.next();
            let var = self.ident()?;
            self.declared.insert(var.clone());
            self.muts.insert(var.clone()); // the loop var is rebindable internally
            if !self.is_kw("in") { return Err("expected `in` in for".to_string()); }
            self.next();
            let lo = self.range_expr()?;
            let inclusive = self.is_p("..=");
            if !self.is_p("..") && !self.is_p("..=") { return Err("expected `..` in for".to_string()); }
            self.next();
            let hi = self.range_expr()?;
            let body = self.block(false)?;
            let cmp = if inclusive { "<=" } else { "<" };
            return Ok(Some(format!("for (int {var} = {lo}; {var} {cmp} {hi}; {var} = {var} + 1) {body}")));
        }
        // assignment: NAME = EXPR ;   (with the immutability check)
        if let Tok::Ident(name) = self.peek().clone() {
            if self.peek2() == &Tok::Punct("=".to_string())
                || self.peek2() == &Tok::Punct("+=".to_string())
                || self.peek2() == &Tok::Punct("-=".to_string())
            {
                let op = if let Tok::Punct(p) = self.peek2().clone() { p } else { String::from("=") };
                if !self.muts.contains(&name) {
                    return Err(format!("cannot assign to immutable binding `{name}` (add `mut`)"));
                }
                self.next(); // name
                self.next(); // = / += / -=
                let e = self.expr()?;
                self.eat_p(";")?;
                return Ok(Some(match op.as_str() {
                    "+=" => format!("{name} = {name} + ({e});"),
                    "-=" => format!("{name} = {name} - ({e});"),
                    _ => format!("{name} = {e};"),
                }));
            }
        }
        Ok(None)
    }

    fn if_stmt(&mut self) -> Result<String, String> {
        self.next(); // if
        let cond = self.expr()?;
        let then = self.block(false)?;
        let mut s = format!("if ({cond}) {then}");
        if self.is_kw("else") {
            self.next();
            if self.is_kw("if") {
                s.push_str(" else ");
                s.push_str(&self.if_stmt()?);
            } else {
                let els = self.block(false)?;
                s.push_str(&format!(" else {els}"));
            }
        }
        Ok(s)
    }

    /// An expression used as a range bound (stops at `..`/`..=`/`{`).
    fn range_expr(&mut self) -> Result<String, String> {
        self.expr_bp(0, true)
    }

    fn expr(&mut self) -> Result<String, String> {
        self.expr_bp(0, false)
    }

    /// Precedence-climbing expression → C text. `in_range` stops before `..`.
    fn expr_bp(&mut self, min_bp: u8, in_range: bool) -> Result<String, String> {
        let mut lhs = self.unary()?;
        loop {
            let (op, bp) = match self.peek() {
                Tok::Punct(p) => {
                    if in_range && (p == ".." || p == "..=") { break; }
                    match p.as_str() {
                        "||" => ("||", 1),
                        "&&" => ("&&", 2),
                        "==" | "!=" | "<" | ">" | "<=" | ">=" => (p.as_str(), 3),
                        "+" | "-" => (p.as_str(), 4),
                        "*" | "/" | "%" => (p.as_str(), 5),
                        _ => break,
                    }
                }
                _ => break,
            };
            if bp < min_bp { break; }
            let opc = op.to_string();
            self.next();
            let rhs = self.expr_bp(bp + 1, in_range)?;
            lhs = format!("({lhs} {opc} {rhs})");
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<String, String> {
        if self.is_p("-") { self.next(); return Ok(format!("(-{})", self.unary()?)); }
        if self.is_p("!") { self.next(); return Ok(format!("(!{})", self.unary()?)); }
        self.primary()
    }

    fn primary(&mut self) -> Result<String, String> {
        // macros: println! / print!
        if let Tok::Ident(name) = self.peek().clone() {
            if (name == "println" || name == "print") && self.peek2() == &Tok::Punct("!".to_string()) {
                return self.print_macro();
            }
        }
        match self.next() {
            Tok::Int(v) => Ok(format!("{v}")),
            Tok::Str(s) => Ok(format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))),
            Tok::Punct(p) if p == "(" => {
                let e = self.expr()?;
                self.eat_p(")")?;
                Ok(format!("({e})"))
            }
            Tok::Ident(name) => {
                if self.is_p("(") {
                    self.next();
                    let mut args = Vec::new();
                    while !self.is_p(")") {
                        args.push(self.expr()?);
                        if self.is_p(",") { self.next(); }
                    }
                    self.eat_p(")")?;
                    Ok(format!("{name}({})", args.join(", ")))
                } else {
                    Ok(name)
                }
            }
            o => Err(format!("unexpected token {o:?}")),
        }
    }

    /// `println!("…", args)` / `print!(…)` -> print("…") / print_int(arg) calls.
    /// We translate to a sequence the C subset prints: a leading literal chunk,
    /// then each `{}` is replaced by a print_int(arg). Returns a 0 placeholder
    /// expression (the prints are emitted as comma-style side effects via cc's
    /// print/print_int builtins, which return void — so we wrap in a call form).
    fn print_macro(&mut self) -> Result<String, String> {
        let nl = matches!(self.peek(), Tok::Ident(s) if s == "println");
        self.next(); // print/println
        self.eat_p("!")?;
        self.eat_p("(")?;
        // optional format string
        let mut fmt = String::new();
        if let Tok::Str(s) = self.peek().clone() { fmt = s; self.next(); }
        let mut args: Vec<String> = Vec::new();
        while self.is_p(",") {
            self.next();
            if self.is_p(")") { break; }
            args.push(self.expr()?);
        }
        self.eat_p(")")?;
        // Build a helper-call chain. The C subset has print("lit") and
        // print_int(x). We emit them as a parenthesised comma sequence; cc treats
        // each as a statement when used as an expr-stmt, so wrap as a 0 value via
        // a synthetic call. Simplest: emit a block-less sequence using the comma
        // operator is unsupported in cc — instead emit them as separate prints by
        // returning a special marker the block layer expands. To keep it simple
        // and within the C subset, translate to nested print_int / print calls
        // chained with `+ 0*` tricks is ugly; instead we emit a call to a
        // generated stub. The pragmatic path: split the format into literal
        // segments around `{}` and emit one print() per literal + one
        // print_int() per arg, joined by the C comma via a helper.
        //
        // The C subset does NOT support the comma operator, so we emit the prints
        // as a single statement only when this macro is used as a statement. We
        // return a marker string that the block layer recognises and expands.
        let mut parts: Vec<String> = Vec::new();
        let mut ai = 0;
        let mut lit = String::new();
        let chars: Vec<char> = fmt.chars().collect();
        let mut k = 0;
        while k < chars.len() {
            if chars[k] == '{' && k + 1 < chars.len() && chars[k + 1] == '}' {
                if !lit.is_empty() { parts.push(format!("print(\"{}\")", lit.replace('"', "\\\""))); lit.clear(); }
                if ai < args.len() { parts.push(format!("print_int({})", args[ai])); ai += 1; }
                k += 2;
            } else {
                lit.push(chars[k]);
                k += 1;
            }
        }
        if !lit.is_empty() { parts.push(format!("print(\"{}\")", lit.replace('"', "\\\""))); }
        // Any extra args with no `{}` (e.g. print!("",x)) just print_int them.
        while ai < args.len() { parts.push(format!("print_int({})", args[ai])); ai += 1; }
        if nl && parts.is_empty() { parts.push("print(\"\\n\")".to_string()); }
        // Marker the block layer turns into separate statements.
        Ok(format!("\u{1}PRINTSEQ\u{1}{}", parts.join("\u{1}")))
    }
}

/// Compile Rust-subset `src` to a WASM module (via the C-subset backend).
pub fn compile(src: &str) -> Result<Vec<u8>, String> {
    let toks = lex(src)?;
    let mut tp = Tp { t: toks, p: 0, muts: BTreeSet::new(), declared: BTreeSet::new() };
    let c = tp.program()?;
    // Expand the print-sequence markers into separate C statements. A line of
    // the form `<SOH>PRINTSEQ<SOH>call1<SOH>call2;` becomes `call1; call2;`.
    let mut out = String::new();
    for line in c.lines() {
        if let Some(pos) = line.find('\u{1}') {
            let indent = &line[..pos];
            let rest = line[pos..].trim_end_matches(';');
            let segs: Vec<&str> = rest.trim_start_matches('\u{1}').split('\u{1}').collect();
            // segs[0] == "PRINTSEQ", the rest are calls
            for call in &segs[1..] {
                if !call.is_empty() {
                    out.push_str(indent);
                    out.push_str(call);
                    out.push_str(";\n");
                }
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    crate::cc::compile(&out)
}

// ---- self-test -------------------------------------------------------------

/// Boot self-test (M42 step 19): compile + run a real Rust program inside Veil.
pub fn selftest() {
    let src = r#"
        fn square(n: i32) -> i32 {
            n * n
        }

        fn sum_to(n: i32) -> i32 {
            let mut total = 0;
            for i in 1..=n {
                total += i;
            }
            total
        }

        fn main() -> i32 {
            println!("Rust on Veil!");
            let a = 7;
            println!("square = {}", square(a));      // 49
            println!("sum = {}", sum_to(10));         // 55
            let mut x = 3;
            if x > 2 {
                x = x * 4;
            } else {
                x = 0;
            }
            println!("x = {}", x);                    // 12
            0
        }
    "#;
    match compile(src) {
        Ok(wasm) => match crate::wasm::run(&wasm) {
            Ok(out) => {
                let o = out.trim().replace('\n', " | ");
                crate::kprintln!("RUSTC: compiled {} bytes of WASM; output: {o}", wasm.len());
                let ok = out.contains("Rust on Veil!")
                    && out.contains("49") && out.contains("55") && out.contains("12");
                if ok {
                    crate::kprintln!("RUSTC_OK: a from-scratch Rust-subset compiler built + ran a real Rust program inside Veil (fn/let-mut/for-range/if-else/println!, immutability checked)");
                } else {
                    crate::kprintln!("RUSTC_FAIL: unexpected output {out:?}");
                }
            }
            Err(e) => crate::kprintln!("RUSTC_FAIL: run error {e}"),
        },
        Err(e) => crate::kprintln!("RUSTC_FAIL: compile error {e}"),
    }

    // Negative proof: the immutability check rejects assigning to a non-`mut`.
    let bad = "fn main() -> i32 { let y = 1; y = 2; y }";
    match compile(bad) {
        Err(e) if e.contains("immutable") => {
            crate::kprintln!("RUSTC_BORROW_OK: rejected assignment to an immutable binding ({e})");
        }
        Err(e) => crate::kprintln!("RUSTC_BORROW: rejected for another reason: {e}"),
        Ok(_) => crate::kprintln!("RUSTC_BORROW_FAIL: immutable reassignment was NOT rejected"),
    }
}
