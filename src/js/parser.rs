//! Recursive-descent JS parser producing the AST. Covers the ES5/ES6 subset
//! the target pages use: var/let/const (+ array destructuring), functions and
//! arrow functions, object/array/template literals, the full expression
//! operator precedence, if/for(/of/in)/while, try/catch, ternary, spread.

use super::ast::*;
use super::lexer::{is_keyword, Tok, TplPart};
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

pub struct Parser {
    t: Vec<Tok>,
    p: usize,
    /// Token-level brace match (idx of `{` -> idx of its `}` and vice versa),
    /// usize::MAX where none. Used only by the desync localizer.
    bm: Vec<usize>,
    /// First `block()` whose `{` closed at a `}` other than its token-level
    /// match — i.e. the first parser/brace desync. usize::MAX = none.
    first_bad: usize,
}

/// Truly-reserved words that can never be an identifier/binding name. This is
/// `is_keyword` minus the *contextual* keywords (`of`, `as`, `get`, `set`,
/// `static`, `async`, `await`, `yield`, `let`, `undefined`) that JS permits as
/// names — minifiers reuse them freely (e.g. `function of(){}`, `var get=…`).
fn is_reserved(s: &str) -> bool {
    is_keyword(s)
        && !matches!(
            s,
            "of" | "as" | "from" | "get" | "set" | "static" | "async" | "await"
                | "yield" | "let" | "undefined"
        )
}

/// Convert an lvalue expression (used as a for-in/for-of target without a
/// declaration keyword) into a binding pattern. Only a bare identifier binds a
/// name; anything else (member access etc.) falls back to a throwaway binding.
fn expr_to_pat(e: Expr) -> Pat {
    match e {
        Expr::Ident(n) => Pat::Ident(n),
        _ => Pat::Ident(String::from("_")),
    }
}

fn brace_match(toks: &[Tok]) -> Vec<usize> {
    let mut bm = alloc::vec![usize::MAX; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, t) in toks.iter().enumerate() {
        if let Tok::Punct(p) = t {
            if *p == "{" {
                stack.push(i);
            } else if *p == "}" {
                if let Some(o) = stack.pop() {
                    bm[o] = i;
                    bm[i] = o;
                }
            }
        }
    }
    bm
}

pub fn parse(toks: Vec<Tok>) -> Vec<Stmt> {
    let mut p = Parser { t: toks, p: 0, bm: Vec::new(), first_bad: usize::MAX };
    let mut out = Vec::new();
    while !p.at_eof() {
        out.push(p.stmt());
    }
    out
}

/// Parse while locating the first brace-nesting desync. Returns the first
/// `block()` open-`{` token index that closed at the wrong `}` (usize::MAX if
/// the parse's block nesting matched the token-level brace structure).
pub fn parse_locate(toks: Vec<Tok>) -> (Vec<Stmt>, usize) {
    let bm = brace_match(&toks);
    let mut p = Parser { t: toks, p: 0, bm, first_bad: usize::MAX };
    let mut out = Vec::new();
    while !p.at_eof() {
        out.push(p.stmt());
    }
    (out, p.first_bad)
}

/// Read the raw token at index i as a short string, for desync diagnostics.
pub fn tok_str(toks: &[Tok], i: usize) -> alloc::string::String {
    match toks.get(i) {
        Some(Tok::Punct(p)) => alloc::string::String::from(*p),
        Some(Tok::Ident(s)) => s.clone(),
        Some(Tok::Num(n)) => alloc::format!("{n}"),
        Some(Tok::Str(_)) => alloc::string::String::from("\"str\""),
        Some(Tok::Regex(..)) => alloc::string::String::from("/re/"),
        Some(Tok::Tmpl(_)) => alloc::string::String::from("`tmpl`"),
        _ => alloc::string::String::from("<eof>"),
    }
}

impl Parser {
    fn at_eof(&self) -> bool {
        matches!(self.t.get(self.p), Some(Tok::Eof) | None)
    }
    fn peek(&self) -> &Tok {
        self.t.get(self.p).unwrap_or(&Tok::Eof)
    }
    fn peek2(&self) -> &Tok {
        self.t.get(self.p + 1).unwrap_or(&Tok::Eof)
    }
    fn bump(&mut self) -> Tok {
        let t = self.t.get(self.p).cloned().unwrap_or(Tok::Eof);
        self.p += 1;
        t
    }
    fn is_punct(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Punct(p) if *p == s)
    }
    fn eat_punct(&mut self, s: &str) -> bool {
        if self.is_punct(s) {
            self.p += 1;
            true
        } else {
            false
        }
    }
    fn expect_punct(&mut self, s: &str) {
        if !self.eat_punct(s) {
            // tolerant: don't hard-fail, just continue
        }
    }
    fn is_kw(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Ident(i) if i == s && is_keyword(i))
    }
    fn eat_kw(&mut self, s: &str) -> bool {
        if self.is_kw(s) {
            self.p += 1;
            true
        } else {
            false
        }
    }

    // ---- statements --------------------------------------------------------

    fn stmt(&mut self) -> Stmt {
        if self.eat_punct(";") {
            return Stmt::Empty;
        }
        if self.is_punct("{") {
            return Stmt::Block(self.block());
        }
        if self.is_kw("var") || self.is_kw("let") || self.is_kw("const") {
            self.bump();
            let d = self.decl_list();
            self.eat_punct(";");
            return Stmt::Decl(d);
        }
        if self.is_kw("function") {
            self.bump();
            return Stmt::FuncDecl(Box::new(self.func(true)));
        }
        // async function declaration
        if matches!(self.peek(), Tok::Ident(i) if i == "async") && matches!(self.peek2(), Tok::Ident(i) if i == "function") {
            self.bump(); // async
            self.bump(); // function
            return Stmt::FuncDecl(Box::new(self.func_flags(true, true)));
        }
        // class declaration
        if matches!(self.peek(), Tok::Ident(i) if i == "class") {
            let c = self.class_expr();
            let name = if let Expr::Class(cb) = &c { cb.name.clone() } else { None };
            if let Some(n) = name {
                // bind the class to its name like a const declaration
                return Stmt::Decl(alloc::vec![(Pat::Ident(n), Some(c))]);
            }
            return Stmt::Expr(c);
        }
        // ES module syntax: import ... ; export ... — we run scripts in one
        // shared global, so treat `export` as a no-op wrapper and skip `import`.
        if matches!(self.peek(), Tok::Ident(i) if i == "import") {
            self.skip_import();
            return Stmt::Empty;
        }
        if matches!(self.peek(), Tok::Ident(i) if i == "export") {
            self.bump(); // export
            if self.eat_kw("default") {
                let e = self.assign();
                self.eat_punct(";");
                return Stmt::Expr(e);
            }
            if matches!(self.peek(), Tok::Punct("{")) {
                // export { a, b } — names already in scope; skip
                self.skip_braces();
                self.eat_punct(";");
                return Stmt::Empty;
            }
            // export const/let/var/function/class … : parse the inner statement
            return self.stmt();
        }
        if self.eat_kw("return") {
            if self.is_punct(";") || self.is_punct("}") || self.at_eof() {
                self.eat_punct(";");
                return Stmt::Return(None);
            }
            let e = self.expr();
            self.eat_punct(";");
            return Stmt::Return(Some(e));
        }
        if self.eat_kw("if") {
            return self.if_stmt();
        }
        if self.eat_kw("for") {
            return self.for_stmt();
        }
        if self.eat_kw("while") {
            self.expect_punct("(");
            let c = self.expr();
            self.expect_punct(")");
            let body = self.body();
            return Stmt::While(c, body);
        }
        if self.eat_kw("do") {
            let body = self.body();
            self.eat_kw("while");
            self.expect_punct("(");
            let c = self.expr();
            self.expect_punct(")");
            self.eat_punct(";");
            // run once then while: model as block + while
            let mut stmts = body.clone();
            stmts.push(Stmt::While(c, body));
            return Stmt::Block(stmts);
        }
        if self.eat_kw("break") {
            // optional label: `break outer;` — consume and ignore the label
            if let Tok::Ident(n) = self.peek().clone() {
                if !is_keyword(&n) { self.bump(); }
            }
            self.eat_punct(";");
            return Stmt::Break;
        }
        if self.eat_kw("continue") {
            if let Tok::Ident(n) = self.peek().clone() {
                if !is_keyword(&n) { self.bump(); }
            }
            self.eat_punct(";");
            return Stmt::Continue;
        }
        if self.eat_kw("throw") {
            let e = self.expr();
            self.eat_punct(";");
            return Stmt::Throw(e);
        }
        if self.eat_kw("try") {
            return self.try_stmt();
        }
        if self.eat_kw("switch") {
            return self.switch_stmt();
        }
        // Labeled statement: `label: stmt` (e.g. minified `a:for(...){...}`).
        // We don't model named break/continue targets, so the label is dropped
        // and `break`/`continue` apply to the innermost loop — fine for the
        // structured loops minifiers emit. The point is to not desync parsing.
        if let Tok::Ident(n) = self.peek().clone() {
            if !is_keyword(&n) && matches!(self.peek2(), Tok::Punct(":")) {
                self.bump(); // label
                self.bump(); // :
                return self.stmt();
            }
        }
        let e = self.expr();
        self.eat_punct(";");
        Stmt::Expr(e)
    }

    fn block(&mut self) -> Vec<Stmt> {
        let open = self.p; // index of the `{`
        self.expect_punct("{");
        let mut out = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            out.push(self.stmt());
        }
        let close = self.p; // index of the `}` we're about to consume
        // Localizer: did this block close at its token-level matching brace?
        if !self.bm.is_empty() && self.first_bad == usize::MAX {
            if self.bm.get(open).copied().unwrap_or(usize::MAX) != close {
                self.first_bad = open;
            }
        }
        self.expect_punct("}");
        out
    }

    /// A statement that may be a single statement or a `{}` block.
    fn body(&mut self) -> Vec<Stmt> {
        if self.is_punct("{") {
            self.block()
        } else {
            alloc::vec![self.stmt()]
        }
    }

    fn decl_list(&mut self) -> Vec<(Pat, Option<Expr>)> {
        let mut out = Vec::new();
        loop {
            let pat = self.pattern();
            let init = if self.eat_punct("=") { Some(self.assign()) } else { None };
            out.push((pat, init));
            if !self.eat_punct(",") {
                break;
            }
        }
        out
    }

    fn pattern(&mut self) -> Pat {
        if self.eat_punct("[") {
            let mut items = Vec::new();
            let mut rest = None;
            while !self.is_punct("]") && !self.at_eof() {
                if self.eat_punct("...") {
                    if let Tok::Ident(n) = self.bump() {
                        rest = Some(n);
                    }
                    break;
                }
                if self.is_punct(",") {
                    // elision: [, x]
                    items.push(Pat::Ident(String::from("_")));
                    self.bump();
                    continue;
                }
                let mut p = self.pattern();
                if self.eat_punct("=") {
                    p = Pat::Default(Box::new(p), Box::new(self.assign()));
                }
                items.push(p);
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct("]");
            Pat::Array(items, rest)
        } else if self.eat_punct("{") {
            // object destructuring: { a, b: c, d = 1, ...rest }
            let mut props = Vec::new();
            let mut rest = None;
            while !self.is_punct("}") && !self.at_eof() {
                if self.eat_punct("...") {
                    if let Tok::Ident(n) = self.bump() {
                        rest = Some(n);
                    }
                    break;
                }
                let key = match self.peek().clone() {
                    Tok::Ident(i) => {
                        self.bump();
                        i
                    }
                    Tok::Str(s) => {
                        self.bump();
                        s
                    }
                    _ => {
                        self.bump();
                        String::from("_")
                    }
                };
                let mut sub = if self.eat_punct(":") {
                    self.pattern()
                } else {
                    Pat::Ident(key.clone())
                };
                if self.eat_punct("=") {
                    sub = Pat::Default(Box::new(sub), Box::new(self.assign()));
                }
                props.push((key, sub));
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct("}");
            Pat::Object(props, rest)
        } else if let Tok::Ident(n) = self.peek().clone() {
            self.bump();
            Pat::Ident(n)
        } else {
            Pat::Ident(String::from("_"))
        }
    }

    fn if_stmt(&mut self) -> Stmt {
        self.expect_punct("(");
        let cond = self.expr();
        self.expect_punct(")");
        let then = self.body();
        let els = if self.eat_kw("else") {
            if self.is_kw("if") {
                self.bump();
                alloc::vec![self.if_stmt()]
            } else {
                self.body()
            }
        } else {
            Vec::new()
        };
        Stmt::If(cond, then, els)
    }

    fn for_stmt(&mut self) -> Stmt {
        self.expect_punct("(");
        // detect for-of / for-in: parse an optional decl/lhs, then check `of`/`in`
        let decl_kw = self.is_kw("var") || self.is_kw("let") || self.is_kw("const");
        if decl_kw {
            self.bump();
            let pat = self.pattern();
            if self.eat_kw("of") {
                let it = self.assign();
                self.expect_punct(")");
                return Stmt::ForOf(pat, it, self.body());
            }
            if self.eat_kw("in") {
                let it = self.assign();
                self.expect_punct(")");
                return Stmt::ForIn(pat, it, self.body());
            }
            // C-style: continue this declarator
            let init = if self.eat_punct("=") { Some(self.assign()) } else { None };
            let mut decls = alloc::vec![(pat, init)];
            while self.eat_punct(",") {
                let p2 = self.pattern();
                let i2 = if self.eat_punct("=") { Some(self.assign()) } else { None };
                decls.push((p2, i2));
            }
            self.expect_punct(";");
            let cond = if self.is_punct(";") { None } else { Some(self.expr()) };
            self.expect_punct(";");
            let upd = if self.is_punct(")") { None } else { Some(self.expr()) };
            self.expect_punct(")");
            return Stmt::For(Box::new(Some(Stmt::Decl(decls))), cond, upd, self.body());
        }
        // no decl keyword
        if self.is_punct(";") {
            self.bump();
            let cond = if self.is_punct(";") { None } else { Some(self.expr()) };
            self.expect_punct(";");
            let upd = if self.is_punct(")") { None } else { Some(self.expr()) };
            self.expect_punct(")");
            return Stmt::For(Box::new(None), cond, upd, self.body());
        }
        let first = self.expr();
        // for-in / for-of with a bare lvalue (no var/let/const), e.g. minified
        // `for(f in b)` / `for(k of arr)`. `expr()` absorbs `f in b` as a binary
        // `in`; `of` is not an operator so it's left as the next token.
        if let Expr::Binary("in", lhs, rhs) = first {
            self.expect_punct(")");
            return Stmt::ForIn(expr_to_pat(*lhs), *rhs, self.body());
        }
        if matches!(self.peek(), Tok::Ident(k) if k == "of") {
            self.bump(); // of
            let it = self.assign();
            self.expect_punct(")");
            return Stmt::ForOf(expr_to_pat(first), it, self.body());
        }
        if matches!(self.peek(), Tok::Ident(k) if k == "in") {
            self.bump(); // in
            let it = self.assign();
            self.expect_punct(")");
            return Stmt::ForIn(expr_to_pat(first), it, self.body());
        }
        self.expect_punct(";");
        let cond = if self.is_punct(";") { None } else { Some(self.expr()) };
        self.expect_punct(";");
        let upd = if self.is_punct(")") { None } else { Some(self.expr()) };
        self.expect_punct(")");
        Stmt::For(Box::new(Some(Stmt::Expr(first))), cond, upd, self.body())
    }

    fn try_stmt(&mut self) -> Stmt {
        let block = self.block();
        let mut catch = None;
        if self.eat_kw("catch") {
            let param = if self.eat_punct("(") {
                let n = if let Tok::Ident(n) = self.peek().clone() {
                    self.bump();
                    Some(n)
                } else {
                    None
                };
                self.expect_punct(")");
                n
            } else {
                None
            };
            catch = Some((param, self.block()));
        }
        let finally = if self.eat_kw("finally") { self.block() } else { Vec::new() };
        Stmt::Try(block, catch, finally)
    }

    fn switch_stmt(&mut self) -> Stmt {
        // Lower switch to if/else-if chain on the discriminant (no fallthrough
        // support beyond the common one-case-per-branch shape).
        self.expect_punct("(");
        let disc = self.expr();
        self.expect_punct(")");
        self.expect_punct("{");
        let mut arms: Vec<(Option<Expr>, Vec<Stmt>)> = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            let test = if self.eat_kw("case") {
                let e = self.expr();
                self.expect_punct(":");
                Some(e)
            } else if self.eat_kw("default") {
                self.expect_punct(":");
                None
            } else {
                self.bump();
                continue;
            };
            let mut body = Vec::new();
            while !self.is_kw("case") && !self.is_kw("default") && !self.is_punct("}") && !self.at_eof() {
                let s = self.stmt();
                if matches!(s, Stmt::Break) {
                    break;
                }
                body.push(s);
            }
            arms.push((test, body));
        }
        self.expect_punct("}");
        // build nested if/else
        let mut chain: Vec<Stmt> = Vec::new();
        let mut default: Vec<Stmt> = Vec::new();
        for (test, body) in arms.iter() {
            if test.is_none() {
                default = body.clone();
            }
        }
        let mut else_branch = default;
        for (test, body) in arms.into_iter().rev() {
            if let Some(t) = test {
                let cond = Expr::Binary("===", Box::new(disc.clone()), Box::new(t));
                else_branch = alloc::vec![Stmt::If(cond, body, core::mem::take(&mut else_branch))];
            }
        }
        chain.extend(else_branch);
        Stmt::Block(chain)
    }

    // ---- classes / modules -------------------------------------------------

    fn class_expr(&mut self) -> Expr {
        self.bump(); // `class`
        let name = if let Tok::Ident(n) = self.peek().clone() {
            if n != "extends" {
                self.bump();
                Some(n)
            } else {
                None
            }
        } else {
            None
        };
        let parent = if matches!(self.peek(), Tok::Ident(i) if i == "extends") {
            self.bump();
            Some(Box::new(self.postfix_no_call()))
        } else {
            None
        };
        self.expect_punct("{");
        let mut ctor = None;
        let mut methods: Vec<(String, Func, bool, &'static str)> = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            if self.eat_punct(";") {
                continue;
            }
            let is_static = matches!(self.peek(), Tok::Ident(i) if i == "static")
                && !matches!(self.peek2(), Tok::Punct("(") | Tok::Punct("="));
            if is_static {
                self.bump();
            }
            let mut kind = "method";
            if let Tok::Ident(g) = self.peek().clone() {
                if (g == "get" || g == "set") && !matches!(self.peek2(), Tok::Punct("(") | Tok::Punct("=")) {
                    self.bump();
                    kind = if g == "get" { "get" } else { "set" };
                }
            }
            let is_async = matches!(self.peek(), Tok::Ident(i) if i == "async")
                && !matches!(self.peek2(), Tok::Punct("(") | Tok::Punct("="));
            if is_async {
                self.bump();
            }
            let is_gen = self.eat_punct("*");
            let mname = self.prop_name();
            // class fields: name = value;  (store as a pseudo-method we run in ctor)
            if self.is_punct("=") {
                self.bump();
                let init = self.assign();
                self.eat_punct(";");
                // model a field initializer as a method named "@field:name"
                let f = Func {
                    name: None, params: Vec::new(),
                    body: alloc::vec![Stmt::Expr(Expr::Assign("=",
                        Box::new(Expr::Member(Box::new(Expr::This), mname.clone(), false)),
                        Box::new(init)))],
                    expr_body: None, arrow: false, is_async: false, is_generator: false,
                };
                methods.push((alloc::format!("@field:{mname}"), f, is_static, "field"));
                continue;
            }
            let params = self.params();
            let body = self.block();
            let f = Func { name: Some(mname.clone()), params, body, expr_body: None, arrow: false, is_async, is_generator: is_gen };
            if mname == "constructor" && kind == "method" && !is_static {
                ctor = Some(f);
            } else {
                methods.push((mname, f, is_static, match kind { "get" => "get", "set" => "set", _ => "method" }));
            }
        }
        self.expect_punct("}");
        Expr::Class(Box::new(Class { name, parent, ctor, methods }))
    }

    fn skip_import(&mut self) {
        // import ... from 'mod';  or  import 'mod';  — consume to the semicolon
        // or end of line's statement. We run all scripts in one global, so the
        // imported names are expected to already be defined.
        self.bump(); // import
        while !self.is_punct(";") && !self.at_eof() {
            if matches!(self.peek(), Tok::Ident(i) if i == "from") {
                self.bump();
                self.bump(); // the module string
                break;
            }
            self.bump();
        }
        self.eat_punct(";");
    }

    fn skip_braces(&mut self) {
        if !self.eat_punct("{") {
            return;
        }
        let mut depth = 1;
        while depth > 0 && !self.at_eof() {
            if self.is_punct("{") {
                depth += 1;
            } else if self.is_punct("}") {
                depth -= 1;
            }
            self.bump();
        }
    }

    // ---- functions ---------------------------------------------------------

    fn func(&mut self, named: bool) -> Func {
        self.func_flags(named, false)
    }

    fn func_flags(&mut self, named: bool, is_async: bool) -> Func {
        let is_generator = self.eat_punct("*");
        let name = if named {
            if let Tok::Ident(n) = self.peek().clone() {
                // Contextual keywords (of/as/get/set/…) are legal function names;
                // minifiers emit them (e.g. `function of(...)` in React).
                if !is_reserved(&n) {
                    self.bump();
                    Some(n)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let params = self.params();
        let body = self.block();
        Func { name, params, body, expr_body: None, arrow: false, is_async, is_generator }
    }

    fn params(&mut self) -> Vec<Pat> {
        self.expect_punct("(");
        let mut out = Vec::new();
        while !self.is_punct(")") && !self.at_eof() {
            if self.eat_punct("...") {
                // rest param: model as array-rest pattern alone
                if let Tok::Ident(n) = self.bump() {
                    out.push(Pat::Array(Vec::new(), Some(n)));
                }
                break;
            }
            let pat = self.pattern();
            // default param value: wrap the binding so the interpreter can fill
            // it when the argument is missing/undefined.
            let pat = if self.eat_punct("=") {
                Pat::Default(Box::new(pat), Box::new(self.assign()))
            } else {
                pat
            };
            out.push(pat);
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")");
        out
    }

    // ---- expressions -------------------------------------------------------

    pub fn expr(&mut self) -> Expr {
        let first = self.assign();
        // comma/sequence operator: evaluate each operand left-to-right (for
        // their side effects) and yield the last. UMD bundles rely on this
        // (`(global = global || self, factory(global.X = {}))`).
        if !self.is_punct(",") {
            return first;
        }
        let mut seq = alloc::vec![first];
        while self.eat_punct(",") {
            seq.push(self.assign());
        }
        Expr::Seq(seq)
    }

    fn assign(&mut self) -> Expr {
        // arrow detection
        if let Some(arrow) = self.try_arrow() {
            return arrow;
        }
        let lhs = self.cond();
        for op in ["=", "+=", "-=", "*=", "/=", "%=", "&&=", "||=", "**=",
                   "|=", "&=", "^=", "<<=", ">>=", ">>>=", "??="].iter() {
            if self.is_punct(op) {
                self.bump();
                let rhs = self.assign();
                return Expr::Assign(op, Box::new(lhs), Box::new(rhs));
            }
        }
        lhs
    }

    /// Recognise `ident =>`, `( params ) =>`, and their `async` forms.
    fn try_arrow(&mut self) -> Option<Expr> {
        // async arrow: `async ident =>` or `async (...) =>`
        if matches!(self.peek(), Tok::Ident(i) if i == "async") {
            if let Tok::Ident(n) = self.peek2().clone() {
                if !is_keyword(&n) && matches!(self.t.get(self.p + 2), Some(Tok::Punct("=>"))) {
                    self.bump(); // async
                    self.bump(); // ident
                    self.bump(); // =>
                    return Some(self.arrow_body_async(alloc::vec![Pat::Ident(n)], true));
                }
            }
            if matches!(self.peek2(), Tok::Punct("(")) {
                let save = self.p;
                self.bump(); // async
                if self.is_punct("(") && self.paren_is_arrow() {
                    let params = self.params();
                    self.expect_punct("=>");
                    return Some(self.arrow_body_async(params, true));
                }
                self.p = save;
            }
        }
        // single ident arrow
        if let Tok::Ident(n) = self.peek().clone() {
            if !is_keyword(&n) && matches!(self.peek2(), Tok::Punct("=>")) {
                self.bump(); // ident
                self.bump(); // =>
                return Some(self.arrow_body(alloc::vec![Pat::Ident(n)]));
            }
        }
        // ( ... ) => : scan ahead for the matching ) then =>
        if self.is_punct("(") && self.paren_is_arrow() {
            let params = self.params();
            self.expect_punct("=>");
            return Some(self.arrow_body(params));
        }
        None
    }

    fn paren_is_arrow(&self) -> bool {
        // from current "(", find matching ")" by depth, then check "=>"
        let mut depth = 0i32;
        let mut i = self.p;
        while i < self.t.len() {
            match self.t.get(i) {
                Some(Tok::Punct("(")) | Some(Tok::Punct("[")) | Some(Tok::Punct("{")) => depth += 1,
                Some(Tok::Punct(")")) | Some(Tok::Punct("]")) | Some(Tok::Punct("}")) => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(self.t.get(i + 1), Some(Tok::Punct("=>")));
                    }
                }
                Some(Tok::Eof) | None => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn arrow_body(&mut self, params: Vec<Pat>) -> Expr {
        self.arrow_body_async(params, false)
    }

    fn arrow_body_async(&mut self, params: Vec<Pat>, is_async: bool) -> Expr {
        if self.is_punct("{") {
            let body = self.block();
            Expr::Arrow(Box::new(Func { name: None, params, body, expr_body: None, arrow: true, is_async, is_generator: false }))
        } else {
            let e = self.assign();
            Expr::Arrow(Box::new(Func { name: None, params, body: Vec::new(), expr_body: Some(Box::new(e)), arrow: true, is_async, is_generator: false }))
        }
    }

    fn cond(&mut self) -> Expr {
        let c = self.binary(0);
        if self.eat_punct("?") {
            let t = self.assign();
            self.expect_punct(":");
            let f = self.assign();
            return Expr::Cond(Box::new(c), Box::new(t), Box::new(f));
        }
        c
    }

    fn binary(&mut self, min_prec: u8) -> Expr {
        let mut left = self.unary();
        loop {
            let (op, prec, logical) = match self.peek() {
                Tok::Punct(p) => match *p {
                    "||" | "??" => (*p, 1, true),
                    "&&" => (*p, 2, true),
                    "|" => (*p, 3, false),
                    "^" => (*p, 4, false),
                    "&" => (*p, 5, false),
                    "==" | "!=" | "===" | "!==" => (*p, 6, false),
                    "<" | ">" | "<=" | ">=" => (*p, 7, false),
                    "<<" | ">>" | ">>>" => (*p, 8, false),
                    "+" | "-" => (*p, 9, false),
                    "*" | "/" | "%" => (*p, 10, false),
                    "**" => (*p, 11, false),
                    _ => break,
                },
                Tok::Ident(i) if i == "instanceof" => ("instanceof", 7, false),
                Tok::Ident(i) if i == "in" => ("in", 7, false),
                _ => break,
            };
            if prec < min_prec {
                break;
            }
            self.bump();
            let right = self.binary(prec + 1);
            left = if logical {
                Expr::Logical(op, Box::new(left), Box::new(right))
            } else {
                Expr::Binary(op, Box::new(left), Box::new(right))
            };
        }
        left
    }

    fn unary(&mut self) -> Expr {
        for op in ["!", "-", "+", "~"].iter() {
            if self.is_punct(op) {
                self.bump();
                return Expr::Unary(op, Box::new(self.unary()));
            }
        }
        if self.is_kw("typeof") {
            self.bump();
            return Expr::Unary("typeof", Box::new(self.unary()));
        }
        if self.is_kw("void") {
            self.bump();
            return Expr::Unary("void", Box::new(self.unary()));
        }
        if self.is_kw("delete") {
            self.bump();
            return Expr::Unary("delete", Box::new(self.unary()));
        }
        // await expr (contextual; treat as a prefix operator)
        if matches!(self.peek(), Tok::Ident(i) if i == "await")
            && !matches!(self.peek2(), Tok::Punct("=>") | Tok::Punct("=") | Tok::Punct(".") | Tok::Punct(";") | Tok::Punct(")"))
        {
            self.bump();
            return Expr::Await(Box::new(self.unary()));
        }
        // yield [*] expr inside a generator
        if matches!(self.peek(), Tok::Ident(i) if i == "yield") {
            self.bump();
            let delegate = self.eat_punct("*");
            if self.is_punct(";") || self.is_punct(")") || self.is_punct("}") || self.is_punct(",") || self.at_eof() {
                return Expr::Yield(None, delegate);
            }
            return Expr::Yield(Some(Box::new(self.assign())), delegate);
        }
        // prefix ++/--
        for op in ["++", "--"].iter() {
            if self.is_punct(op) {
                self.bump();
                return Expr::Update(op, true, Box::new(self.unary()));
            }
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Expr {
        let mut e = self.primary();
        loop {
            if self.eat_punct(".") {
                let name = self.ident_name();
                e = Expr::Member(Box::new(e), name, false);
            } else if self.is_punct("?.") {
                self.bump();
                if self.is_punct("(") {
                    let args = self.args();
                    e = Expr::Call(Box::new(e), args, true);
                } else {
                    let name = self.ident_name();
                    e = Expr::Member(Box::new(e), name, true);
                }
            } else if self.eat_punct("[") {
                let idx = self.expr();
                self.expect_punct("]");
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else if self.is_punct("(") {
                let args = self.args();
                e = Expr::Call(Box::new(e), args, false);
            } else if self.is_punct("++") || self.is_punct("--") {
                let op = if self.is_punct("++") { "++" } else { "--" };
                self.bump();
                e = Expr::Update(op, false, Box::new(e));
            } else {
                break;
            }
        }
        e
    }

    fn ident_name(&mut self) -> String {
        match self.bump() {
            Tok::Ident(n) => n,
            _ => String::from("undefined"),
        }
    }

    fn args(&mut self) -> Vec<Expr> {
        self.expect_punct("(");
        let mut out = Vec::new();
        while !self.is_punct(")") && !self.at_eof() {
            if self.eat_punct("...") {
                out.push(Expr::Spread(Box::new(self.assign())));
            } else {
                out.push(self.assign());
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")");
        out
    }

    fn primary(&mut self) -> Expr {
        match self.peek().clone() {
            Tok::Num(n) => {
                self.bump();
                Expr::Num(n)
            }
            Tok::Str(s) => {
                self.bump();
                Expr::Str(s)
            }
            Tok::Tmpl(parts) => {
                self.bump();
                let mut elems = Vec::new();
                for part in parts {
                    match part {
                        TplPart::Str(s) => elems.push(TplElem::Str(s)),
                        TplPart::Expr(toks) => {
                            let mut sub = Parser { t: toks, p: 0, bm: Vec::new(), first_bad: usize::MAX };
                            elems.push(TplElem::Expr(Box::new(sub.expr())));
                        }
                    }
                }
                Expr::Tmpl(elems)
            }
            Tok::Regex(pat, flags) => {
                self.bump();
                Expr::Regex(pat, flags)
            }
            Tok::Punct("(") => {
                self.bump();
                let e = self.expr();
                self.expect_punct(")");
                e
            }
            Tok::Punct("[") => {
                self.bump();
                let mut items = Vec::new();
                while !self.is_punct("]") && !self.at_eof() {
                    if self.is_punct(",") {
                        self.bump();
                        items.push(Expr::Undef);
                        continue;
                    }
                    if self.eat_punct("...") {
                        items.push(Expr::Spread(Box::new(self.assign())));
                    } else {
                        items.push(self.assign());
                    }
                    if !self.eat_punct(",") {
                        break;
                    }
                }
                self.expect_punct("]");
                Expr::Array(items)
            }
            Tok::Punct("{") => self.object(),
            Tok::Ident(id) => {
                match id.as_str() {
                    "true" => {
                        self.bump();
                        Expr::Bool(true)
                    }
                    "false" => {
                        self.bump();
                        Expr::Bool(false)
                    }
                    "null" => {
                        self.bump();
                        Expr::Null
                    }
                    "undefined" => {
                        self.bump();
                        Expr::Undef
                    }
                    "this" => {
                        self.bump();
                        Expr::This
                    }
                    "function" => {
                        self.bump();
                        Expr::Func(Box::new(self.func(true)))
                    }
                    "async" if matches!(self.peek2(), Tok::Ident(i) if i == "function") => {
                        self.bump(); // async
                        self.bump(); // function
                        Expr::Func(Box::new(self.func_flags(true, true)))
                    }
                    "class" => self.class_expr(),
                    "super" => {
                        self.bump();
                        if self.eat_punct(".") {
                            let name = self.ident_name();
                            Expr::Super(Some(name))
                        } else {
                            Expr::Super(None)
                        }
                    }
                    "new" => {
                        self.bump();
                        if matches!(self.peek(), Tok::Punct(".")) {
                            // new.target — not supported, evaluate to undefined
                            self.bump();
                            let _ = self.ident_name();
                            return Expr::Undef;
                        }
                        let callee = self.postfix_no_call();
                        let args = if self.is_punct("(") { self.args() } else { Vec::new() };
                        Expr::New(Box::new(callee), args)
                    }
                    _ => {
                        self.bump();
                        Expr::Ident(id)
                    }
                }
            }
            _ => {
                self.bump();
                Expr::Undef
            }
        }
    }

    /// For `new X.Y(...)`: parse member chain but stop before the call args.
    fn postfix_no_call(&mut self) -> Expr {
        let mut e = self.primary();
        loop {
            if self.eat_punct(".") {
                let name = self.ident_name();
                e = Expr::Member(Box::new(e), name, false);
            } else if self.eat_punct("[") {
                let idx = self.expr();
                self.expect_punct("]");
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else {
                break;
            }
        }
        e
    }

    fn object(&mut self) -> Expr {
        self.expect_punct("{");
        let mut props = Vec::new();
        while !self.is_punct("}") && !self.at_eof() {
            // object spread: ...x
            if self.eat_punct("...") {
                let e = self.assign();
                props.push((PropKey::Spread, e));
                self.eat_punct(",");
                continue;
            }
            // getter/setter shorthand: get name() {}, set name(v) {}
            if let Tok::Ident(g) = self.peek().clone() {
                if (g == "get" || g == "set")
                    && !matches!(self.peek2(), Tok::Punct(":") | Tok::Punct(",") | Tok::Punct("(") | Tok::Punct("}"))
                {
                    self.bump(); // get/set
                    let name = self.prop_name();
                    let params = self.params();
                    let body = self.block();
                    let f = Func { name: None, params, body, expr_body: None, arrow: false, is_async: false, is_generator: false };
                    let key = if g == "get" { PropKey::Getter(name) } else { PropKey::Setter(name) };
                    props.push((key, Expr::Func(Box::new(f))));
                    self.eat_punct(",");
                    continue;
                }
            }
            // async / generator method shorthand
            let is_async = matches!(self.peek(), Tok::Ident(i) if i == "async")
                && !matches!(self.peek2(), Tok::Punct(":") | Tok::Punct(",") | Tok::Punct("(") | Tok::Punct("}"));
            if is_async {
                self.bump();
            }
            let is_gen = self.eat_punct("*");
            let key = match self.peek().clone() {
                Tok::Str(s) => {
                    self.bump();
                    PropKey::Ident(s)
                }
                Tok::Num(n) => {
                    self.bump();
                    PropKey::Ident(super::value::num_to_str(n))
                }
                Tok::Ident(i) => {
                    self.bump();
                    PropKey::Ident(i)
                }
                Tok::Punct("[") => {
                    self.bump();
                    let e = self.assign();
                    self.expect_punct("]");
                    PropKey::Computed(Box::new(e))
                }
                _ => {
                    self.bump();
                    PropKey::Ident(String::from("_"))
                }
            };
            let val = if self.eat_punct(":") {
                self.assign()
            } else if self.is_punct("(") {
                // method shorthand
                let params = self.params();
                let body = self.block();
                Expr::Func(Box::new(Func { name: None, params, body, expr_body: None, arrow: false, is_async, is_generator: is_gen }))
            } else {
                // shorthand { foo } or { foo = default } (destructuring-ish)
                if self.eat_punct("=") {
                    let _ = self.assign();
                }
                match &key {
                    PropKey::Ident(n) => Expr::Ident(n.clone()),
                    _ => Expr::Undef,
                }
            };
            props.push((key, val));
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct("}");
        Expr::Object(props)
    }

    fn prop_name(&mut self) -> String {
        match self.bump() {
            Tok::Ident(n) => n,
            Tok::Str(s) => s,
            Tok::Num(n) => super::value::num_to_str(n),
            _ => String::from("_"),
        }
    }
}
