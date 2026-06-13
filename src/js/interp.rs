//! Tree-walking JS evaluator with scopes, closures, and the host bindings
//! (document / window / console / Math / localStorage, DOM elements, and the
//! Array/String/Object methods) the target pages use.

use super::ast::*;
use super::dom::Dom;
use super::value::{num_to_str, Host, Native, Obj, Val};
use super::mathf;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use alloc::vec;
use core::cell::RefCell;

pub type Scope = Rc<RefCell<Env>>;

pub struct Env {
    pub vars: BTreeMap<String, Val>,
    pub parent: Option<Scope>,
}

fn new_scope(parent: Option<Scope>) -> Scope {
    Rc::new(RefCell::new(Env { vars: BTreeMap::new(), parent }))
}

enum Flow {
    Normal,
    Return(Val),
    Break,
    Continue,
}

/// A registered event listener: (node, event-type, handler).
pub struct Listener {
    pub node: usize,
    pub event: String,
    pub handler: Val,
}

pub struct Interp {
    pub dom: Dom,
    pub global: Scope,
    storage: BTreeMap<String, String>,
    deferred: Vec<(Val, Vec<Val>)>,
    pub listeners: Vec<Listener>,
    pub errors: Vec<String>,
    steps: u64,
    /// Promise resolve/reject cells, indexed by the id baked into the
    /// `__resolve:N` / `__reject:N` native function name.
    resolvers: Vec<Rc<RefCell<(String, Val)>>>,
    /// Profile-guided JIT: per-function call counts and compiled native code.
    /// Keyed by the Rc<Func> address; the pinned Rc clone keeps that allocation
    /// alive so the address can't be reused by a *different* function while it's
    /// cached (which would otherwise alias a stale compile — an ABA bug).
    jit: BTreeMap<usize, (Rc<Func>, JitSlot)>,
    jit_enabled: bool,
    /// Per-function property bag (keyed by Rc<Func> address): holds `.prototype`
    /// and any static properties assigned to a function object. This gives the
    /// pre-ES6 prototype model (`Ctor.prototype.method = …`; `new Ctor()`
    /// instances inherit via a hidden `__proto__` link) that minified UMD
    /// libraries (React's `ReactDOMRoot.prototype.render`) rely on.
    func_props: BTreeMap<usize, (Rc<Func>, Rc<RefCell<Obj>>)>,
    /// 2D canvas contexts created via `<canvas>.getContext('2d')`; the owning
    /// element stores its index in a `__cvs` attribute so the browser can blit
    /// the drawn buffer where the canvas sits in layout.
    pub canvases: Vec<super::canvas::Canvas>,
    /// WebGL rendering contexts created via `<canvas>.getContext('webgl')`. Each
    /// owns GL state + a GLSL software rasteriser; it renders into the canvas at
    /// `gl.canvas` (an index into `canvases`) so the result shows in layout.
    pub webgl: Vec<super::webgl::GlContext>,
}

/// Cap on distinct cached functions before the whole table is dropped (bounds
/// memory for pages that mint many short-lived closures).
const JIT_CACHE_CAP: usize = 4096;

/// Diagnostic counter for `document.createElement` calls (React commit tracing).
pub static CE_COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

enum JitSlot {
    Profiling(u32),
    Bailed,
    Compiled(Rc<super::jit::Code>),
}

/// Interpret first, then JIT after this many calls (profile-guided).
const JIT_THRESHOLD: u32 = 1;

impl Interp {
    pub fn new(dom: Dom) -> Interp {
        let global = new_scope(None);
        let mut it = Interp {
            dom,
            global: global.clone(),
            storage: BTreeMap::new(),
            deferred: Vec::new(),
            listeners: Vec::new(),
            errors: Vec::new(),
            steps: 0,
            resolvers: Vec::new(),
            jit: BTreeMap::new(),
            jit_enabled: true,
            func_props: BTreeMap::new(),
            canvases: Vec::new(),
            webgl: Vec::new(),
        };
        it.install_globals();
        it
    }

    fn install_globals(&mut self) {
        let g = self.global.clone();
        let mut b = g.borrow_mut();
        b.vars.insert("document".into(), Val::Host(Host::Document));
        b.vars.insert("window".into(), Val::Host(Host::Window));
        b.vars.insert("self".into(), Val::Host(Host::Window));
        b.vars.insert("globalThis".into(), Val::Host(Host::Window));
        b.vars.insert("console".into(), Val::Host(Host::Console));
        b.vars.insert("Math".into(), Val::Host(Host::Math));
        b.vars.insert("localStorage".into(), Val::Host(Host::LocalStorage));
        b.vars.insert("sessionStorage".into(), Val::Host(Host::SessionStorage));
        b.vars.insert("history".into(), Val::Host(Host::History));
        b.vars.insert("location".into(), Val::Host(Host::Location));
        for f in ["setTimeout", "setInterval", "requestAnimationFrame", "clearTimeout",
                  "clearInterval", "cancelAnimationFrame", "parseInt", "parseFloat", "isNaN",
                  "isFinite", "String", "Number", "Boolean", "Array", "Object", "JSON",
                  "encodeURIComponent", "decodeURIComponent", "encodeURI", "decodeURI",
                  "alert", "fetch", "addEventListener", "structuredClone", "queueMicrotask",
                  "__webaudio_play", "__webaudio_decode",
                  // ES6 constructors / namespaces (callable + member access via
                  // get_member returning "Name.prop" natives).
                  "Promise", "Map", "Set", "WeakMap", "WeakSet", "Symbol", "Date",
                  "RegExp", "Error", "TypeError", "RangeError", "SyntaxError",
                  "ReferenceError", "Reflect", "Proxy", "BigInt", "WebSocket",
                  // M42 step 18 (V8-parity): web platform constructors/namespaces.
                  "URL", "URLSearchParams", "TextEncoder", "TextDecoder", "FormData",
                  "Blob", "File", "WeakRef", "FinalizationRegistry", "EvalError",
                  // M42 step 17: registered so `typeof MessageChannel === 'function'`
                  // (React's scheduler probes this to pick its work-loop transport).
                  "MessageChannel", "Event", "CustomEvent", "MutationObserver"] {
            b.vars.insert(f.into(), Val::Native(Native::Global(Rc::from(f))));
        }
        // crypto.getRandomValues / randomUUID.
        {
            let mut c = Obj::new();
            c.insert("getRandomValues".into(), Val::Native(Native::Global(Rc::from("crypto.getRandomValues"))));
            c.insert("randomUUID".into(), Val::Native(Native::Global(Rc::from("crypto.randomUUID"))));
            b.vars.insert("crypto".into(), Val::object(c));
        }
        b.vars.insert("NaN".into(), Val::Num(f64::NAN));
        b.vars.insert("Infinity".into(), Val::Num(f64::INFINITY));
        b.vars.insert("undefined".into(), Val::Undef);
        // performance.now() — a high-res timer some libraries probe for.
        {
            let mut p = Obj::new();
            p.insert("now".into(), Val::Native(Native::Global(Rc::from("performance.now"))));
            b.vars.insert("performance".into(), Val::object(p));
        }
        // navigator — userAgent / language sniffed by many libraries.
        {
            let mut nav = Obj::new();
            nav.insert("userAgent".into(), Val::str("Mozilla/5.0 (Veil OS) VeilBrowser/1.0"));
            nav.insert("language".into(), Val::str("en-US"));
            nav.insert("languages".into(), Val::array(alloc::vec![Val::str("en-US"), Val::str("en")]));
            nav.insert("onLine".into(), Val::Bool(true));
            nav.insert("platform".into(), Val::str("Veil"));
            b.vars.insert("navigator".into(), Val::object(nav));
        }
    }

    /// Run a script's source against the current DOM. Errors are recorded, not
    /// propagated, so a broken script doesn't abort the others.
    pub fn run(&mut self, src: &str) {
        let toks = super::lexer::tokenize(src);
        let prog = super::parser::parse(toks);
        let scope = self.global.clone();
        // hoist function declarations
        self.hoist(&prog, &scope);
        for st in &prog {
            if let Err(e) = self.exec(st, &scope) {
                self.errors.push(alloc::format!("uncaught: {}", e.to_str()));
                break;
            }
        }
    }

    /// Run any deferred callbacks (setTimeout/requestAnimationFrame) queued
    /// during the scripts, bounded so a self-rescheduling loop can't hang.
    pub fn drain_deferred(&mut self) {
        let mut rounds = 0;
        // Bounded so a self-rescheduling callback (rAF/microtask) can't hang the
        // render. 200 rounds is enough for libraries that flush work in waves.
        while !self.deferred.is_empty() && rounds < 200 {
            let batch = core::mem::take(&mut self.deferred);
            for (f, args) in batch {
                let _ = self.call(f, Val::Undef, args);
            }
            rounds += 1;
        }
    }

    fn hoist(&mut self, stmts: &[Stmt], scope: &Scope) {
        for st in stmts {
            if let Stmt::FuncDecl(f) = st {
                if let Some(name) = &f.name {
                    let v = Val::Func(rc_func(f), scope.clone());
                    scope.borrow_mut().vars.insert(name.clone(), v);
                }
            }
        }
    }

    // ---- statement execution ----------------------------------------------

    fn exec_block(&mut self, stmts: &[Stmt], scope: &Scope) -> Result<Flow, Val> {
        self.hoist(stmts, scope);
        for st in stmts {
            match self.exec(st, scope)? {
                Flow::Normal => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Normal)
    }

    fn exec(&mut self, st: &Stmt, scope: &Scope) -> Result<Flow, Val> {
        self.steps += 1;
        if self.steps > 5_000_000 {
            return Err(Val::str("script step limit"));
        }
        match st {
            Stmt::Empty | Stmt::FuncDecl(_) => Ok(Flow::Normal),
            Stmt::Expr(e) => {
                self.eval(e, scope)?;
                Ok(Flow::Normal)
            }
            Stmt::Decl(decls) => {
                for (pat, init) in decls {
                    let v = match init {
                        Some(e) => self.eval(e, scope)?,
                        None => Val::Undef,
                    };
                    self.bind_pat(pat, v, scope)?;
                }
                Ok(Flow::Normal)
            }
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e, scope)?,
                    None => Val::Undef,
                };
                Ok(Flow::Return(v))
            }
            Stmt::If(c, t, e) => {
                if self.eval(c, scope)?.truthy() {
                    let inner = new_scope(Some(scope.clone()));
                    self.exec_block(t, &inner)
                } else {
                    let inner = new_scope(Some(scope.clone()));
                    self.exec_block(e, &inner)
                }
            }
            Stmt::Block(b) => {
                let inner = new_scope(Some(scope.clone()));
                self.exec_block(b, &inner)
            }
            Stmt::While(c, body) => {
                while self.eval(c, scope)?.truthy() {
                    let inner = new_scope(Some(scope.clone()));
                    match self.exec_block(body, &inner)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    self.steps += 1;
                    if self.steps > 5_000_000 {
                        return Err(Val::str("loop limit"));
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::For(init, cond, upd, body) => {
                let outer = new_scope(Some(scope.clone()));
                if let Some(s) = init.as_ref() {
                    self.exec(s, &outer)?;
                }
                loop {
                    if let Some(c) = cond {
                        if !self.eval(c, &outer)?.truthy() {
                            break;
                        }
                    }
                    let inner = new_scope(Some(outer.clone()));
                    match self.exec_block(body, &inner)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                    if let Some(u) = upd {
                        self.eval(u, &outer)?;
                    }
                    self.steps += 1;
                    if self.steps > 5_000_000 {
                        return Err(Val::str("loop limit"));
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::ForOf(pat, iter, body) => {
                let items = self.iterate(iter, scope)?;
                for it in items {
                    let inner = new_scope(Some(scope.clone()));
                    self.bind_pat(pat, it, &inner)?;
                    match self.exec_block(body, &inner)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::ForIn(pat, iter, body) => {
                let obj = self.eval(iter, scope)?;
                let keys: Vec<Val> = match obj {
                    Val::Object(o) => o.borrow().keys().map(|k| Val::str(k.clone())).collect(),
                    Val::Array(a) => (0..a.borrow().len()).map(|i| Val::str(i.to_string())).collect(),
                    _ => Vec::new(),
                };
                for k in keys {
                    let inner = new_scope(Some(scope.clone()));
                    self.bind_pat(pat, k, &inner)?;
                    match self.exec_block(body, &inner)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        _ => {}
                    }
                }
                Ok(Flow::Normal)
            }
            Stmt::Break => Ok(Flow::Break),
            Stmt::Continue => Ok(Flow::Continue),
            Stmt::Throw(e) => {
                let v = self.eval(e, scope)?;
                Err(v)
            }
            Stmt::Try(block, catch, finally) => {
                let inner = new_scope(Some(scope.clone()));
                let r = self.exec_block(block, &inner);
                let out = match r {
                    Err(ex) => {
                        if let Some((param, cbody)) = catch {
                            let cs = new_scope(Some(scope.clone()));
                            if let Some(p) = param {
                                cs.borrow_mut().vars.insert(p.clone(), ex);
                            }
                            self.exec_block(cbody, &cs)
                        } else {
                            Err(ex)
                        }
                    }
                    ok => ok,
                };
                if !finally.is_empty() {
                    let fs = new_scope(Some(scope.clone()));
                    self.exec_block(finally, &fs)?;
                }
                out
            }
        }
    }

    fn bind_pat(&mut self, pat: &Pat, val: Val, scope: &Scope) -> Result<(), Val> {
        match pat {
            Pat::Ident(n) => {
                scope.borrow_mut().vars.insert(n.clone(), val);
            }
            Pat::Default(inner, def) => {
                let v = if matches!(val, Val::Undef) { self.eval(def, scope)? } else { val };
                self.bind_pat(inner, v, scope)?;
            }
            Pat::Array(items, rest) => {
                let arr = self.to_vec(&val);
                for (i, p) in items.iter().enumerate() {
                    self.bind_pat(p, arr.get(i).cloned().unwrap_or(Val::Undef), scope)?;
                }
                if let Some(r) = rest {
                    let tail: Vec<Val> = arr.into_iter().skip(items.len()).collect();
                    scope.borrow_mut().vars.insert(r.clone(), Val::array(tail));
                }
            }
            Pat::Object(props, rest) => {
                let mut taken: Vec<String> = Vec::new();
                for (key, sub) in props {
                    let v = self.get_member(val.clone(), key)?;
                    taken.push(key.clone());
                    self.bind_pat(sub, v, scope)?;
                }
                if let Some(r) = rest {
                    // rest collects the remaining own enumerable props.
                    let mut o = Obj::new();
                    if let Val::Object(m) = &val {
                        for (k, v) in m.borrow().iter() {
                            if !taken.contains(k) && !k.starts_with("__") {
                                o.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    scope.borrow_mut().vars.insert(r.clone(), Val::object(o));
                }
            }
        }
        Ok(())
    }

    fn iterate(&mut self, e: &Expr, scope: &Scope) -> Result<Vec<Val>, Val> {
        let v = self.eval(e, scope)?;
        Ok(self.to_vec(&v))
    }

    fn to_vec(&self, v: &Val) -> Vec<Val> {
        match v {
            Val::Array(a) => a.borrow().clone(),
            Val::Str(s) => s.chars().map(|c| Val::str(c.to_string())).collect(),
            Val::Node(idx) => self.dom.nodes[*idx].children.iter().map(|&c| Val::Node(c)).collect(),
            _ => Vec::new(),
        }
    }

    // ---- expression evaluation ---------------------------------------------

    fn eval(&mut self, e: &Expr, scope: &Scope) -> Result<Val, Val> {
        match e {
            Expr::Num(n) => Ok(Val::Num(*n)),
            Expr::Str(s) => Ok(Val::str(s.clone())),
            Expr::Bool(b) => Ok(Val::Bool(*b)),
            Expr::Null => Ok(Val::Null),
            Expr::Undef => Ok(Val::Undef),
            Expr::This => Ok(self.lookup(scope, "this").unwrap_or(Val::Undef)),
            Expr::Ident(n) => Ok(self.lookup(scope, n).unwrap_or(Val::Undef)),
            Expr::Tmpl(parts) => {
                let mut s = String::new();
                for p in parts {
                    match p {
                        TplElem::Str(t) => s.push_str(t),
                        TplElem::Expr(e) => s.push_str(&self.eval(e, scope)?.to_str()),
                    }
                }
                Ok(Val::str(s))
            }
            Expr::Array(items) => {
                let mut out = Vec::new();
                for it in items {
                    if let Expr::Spread(inner) = it {
                        let v = self.eval(inner, scope)?;
                        out.extend(self.to_vec(&v));
                    } else {
                        out.push(self.eval(it, scope)?);
                    }
                }
                Ok(Val::array(out))
            }
            Expr::Object(props) => {
                let mut o = Obj::new();
                for (k, v) in props {
                    match k {
                        PropKey::Spread => {
                            let src = self.eval(v, scope)?;
                            match src {
                                Val::Object(m) => {
                                    for (kk, vv) in m.borrow().iter() {
                                        if !kk.starts_with("__") {
                                            o.insert(kk.clone(), vv.clone());
                                        }
                                    }
                                }
                                Val::Array(a) => {
                                    for (i, vv) in a.borrow().iter().enumerate() {
                                        o.insert(i.to_string(), vv.clone());
                                    }
                                }
                                _ => {}
                            }
                        }
                        PropKey::Getter(name) => {
                            let f = self.eval(v, scope)?;
                            o.insert(alloc::format!("__get:{name}"), f);
                        }
                        PropKey::Setter(name) => {
                            let f = self.eval(v, scope)?;
                            o.insert(alloc::format!("__set:{name}"), f);
                        }
                        PropKey::Ident(s) => {
                            let val = self.eval(v, scope)?;
                            o.insert(s.clone(), val);
                        }
                        PropKey::Computed(e) => {
                            let key = self.eval(e, scope)?.to_str();
                            let val = self.eval(v, scope)?;
                            o.insert(key, val);
                        }
                    }
                }
                Ok(Val::object(o))
            }
            Expr::Spread(e) => self.eval(e, scope),
            Expr::Func(f) => Ok(Val::Func(rc_func(f), scope.clone())),
            Expr::Arrow(f) => Ok(Val::Func(rc_func(f), scope.clone())),
            Expr::Unary(op, e) => {
                if *op == "typeof" {
                    let v = match self.eval(e, scope) {
                        Ok(v) => v,
                        Err(_) => Val::Undef,
                    };
                    return Ok(Val::str(type_of(&v)));
                }
                let v = self.eval(e, scope)?;
                Ok(match *op {
                    "!" => Val::Bool(!v.truthy()),
                    "-" => Val::Num(-v.as_num()),
                    "+" => Val::Num(v.as_num()),
                    "~" => Val::Num(!to_int32(v.as_num()) as f64),
                    "void" => Val::Undef,
                    "delete" => Val::Bool(true),
                    _ => Val::Undef,
                })
            }
            Expr::Update(op, prefix, target) => {
                let old = self.eval(target, scope)?.as_num();
                let new = if *op == "++" { old + 1.0 } else { old - 1.0 };
                self.assign_to(target, Val::Num(new), scope)?;
                Ok(Val::Num(if *prefix { new } else { old }))
            }
            Expr::Binary(op, a, b) => {
                let l = self.eval(a, scope)?;
                let r = self.eval(b, scope)?;
                if *op == "instanceof" {
                    return Ok(Val::Bool(self.instance_of(&l, &r)));
                }
                Ok(binop(op, l, r))
            }
            Expr::Logical(op, a, b) => {
                let l = self.eval(a, scope)?;
                match *op {
                    "&&" => {
                        if l.truthy() {
                            self.eval(b, scope)
                        } else {
                            Ok(l)
                        }
                    }
                    "||" => {
                        if l.truthy() {
                            Ok(l)
                        } else {
                            self.eval(b, scope)
                        }
                    }
                    "??" => {
                        if matches!(l, Val::Undef | Val::Null) {
                            self.eval(b, scope)
                        } else {
                            Ok(l)
                        }
                    }
                    _ => Ok(Val::Undef),
                }
            }
            Expr::Cond(c, t, f) => {
                if self.eval(c, scope)?.truthy() {
                    self.eval(t, scope)
                } else {
                    self.eval(f, scope)
                }
            }
            Expr::Assign(op, target, value) => {
                let v = if *op == "=" {
                    self.eval(value, scope)?
                } else {
                    let cur = self.eval(target, scope)?;
                    let rhs = self.eval(value, scope)?;
                    let base = &op[..op.len() - 1];
                    binop(base, cur, rhs)
                };
                self.assign_to(target, v.clone(), scope)?;
                Ok(v)
            }
            Expr::Seq(parts) => {
                let mut last = Val::Undef;
                for p in parts {
                    last = self.eval(p, scope)?;
                }
                Ok(last)
            }
            Expr::Regex(pat, flags) => {
                let mut o = Obj::new();
                o.insert("__regex".into(), Val::Bool(true));
                o.insert("source".into(), Val::str(pat.clone()));
                o.insert("flags".into(), Val::str(flags.clone()));
                o.insert("global".into(), Val::Bool(flags.contains('g')));
                o.insert("lastIndex".into(), Val::Num(0.0));
                Ok(Val::object(o))
            }
            Expr::Member(obj, prop, opt) => {
                let o = self.eval(obj, scope)?;
                if *opt && matches!(o, Val::Undef | Val::Null) {
                    return Ok(Val::Undef);
                }
                self.get_member(o, prop)
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(obj, scope)?;
                let i = self.eval(idx, scope)?;
                self.get_index(o, i)
            }
            Expr::Call(callee, args, opt) => self.eval_call(callee, args, *opt, scope),
            Expr::New(callee, args) => {
                let ctor = self.eval(callee, scope)?;
                let ctor_name = match &**callee {
                    Expr::Ident(n) => n.clone(),
                    Expr::Member(_, p, _) => p.clone(),
                    _ => String::new(),
                };
                let argv = self.eval_args(args, scope)?;
                self.construct(ctor, &ctor_name, argv)
            }
            Expr::Await(e) => {
                let v = self.eval(e, scope)?;
                self.await_val(v)
            }
            Expr::Yield(e, _) => match e {
                Some(e) => self.eval(e, scope),
                None => Ok(Val::Undef),
            },
            Expr::Class(c) => self.eval_class(c, scope),
            Expr::Super(prop) => {
                // bare `super` value: return the superclass; super.prop read.
                let sc = self.current_super(scope);
                match prop {
                    None => Ok(sc),
                    Some(p) => {
                        let m = self.class_lookup_method(&sc, p);
                        Ok(m.unwrap_or(Val::Undef))
                    }
                }
            }
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr], opt: bool, scope: &Scope) -> Result<Val, Val> {
        // super(...) and super.method(...) inside a class
        if let Expr::Super(prop) = callee {
            let argv = self.eval_args(args, scope)?;
            let this = self.lookup(scope, "this").unwrap_or(Val::Undef);
            let sc = self.current_super(scope);
            match prop {
                None => {
                    // super(...) — run the parent constructor chain on `this`.
                    self.run_ctor_chain(&sc, this, argv)?;
                    return Ok(Val::Undef);
                }
                Some(p) => {
                    if let Some(m) = self.class_lookup_method(&sc, p) {
                        return self.call(m, this, argv);
                    }
                    return Ok(Val::Undef);
                }
            }
        }
        // method calls: capture receiver
        let (func, this) = match callee {
            Expr::Member(obj, prop, mopt) => {
                let recv = self.eval(obj, scope)?;
                if (*mopt || opt) && matches!(recv, Val::Undef | Val::Null) {
                    return Ok(Val::Undef);
                }
                // Try a builtin method first (returns the result directly).
                let argv = self.eval_args(args, scope)?;
                if let Some(r) = self.builtin_method(&recv, prop, &argv)? {
                    return Ok(r);
                }
                let f = self.get_member(recv.clone(), prop)?;
                return self.call(f, recv, argv);
            }
            _ => {
                let f = self.eval(callee, scope)?;
                (f, Val::Undef)
            }
        };
        if opt && matches!(func, Val::Undef | Val::Null) {
            return Ok(Val::Undef);
        }
        let argv = self.eval_args(args, scope)?;
        self.call(func, this, argv)
    }

    fn eval_args(&mut self, args: &[Expr], scope: &Scope) -> Result<Vec<Val>, Val> {
        let mut out = Vec::new();
        for a in args {
            if let Expr::Spread(inner) = a {
                let v = self.eval(inner, scope)?;
                out.extend(self.to_vec(&v));
            } else {
                out.push(self.eval(a, scope)?);
            }
        }
        Ok(out)
    }

    /// If `f` is (or becomes) a JIT-compiled numeric function and every argument
    /// is a number, run the native code and return its result. None = deopt to
    /// the interpreter.
    pub fn set_jit(&mut self, on: bool) {
        self.jit_enabled = on;
    }

    /// Read a global binding (used by the JIT self-test to fetch a function).
    pub fn global_val(&self, name: &str) -> Option<Val> {
        self.global.borrow().vars.get(name).cloned()
    }

    fn try_jit(&mut self, f: &Rc<Func>, args: &[Val]) -> Option<Val> {
        if !self.jit_enabled {
            return None;
        }
        if self.jit.len() > JIT_CACHE_CAP {
            self.jit.clear(); // drop all pins; functions will re-profile
        }
        let key = Rc::as_ptr(f) as *const () as usize;
        enum Act {
            Run(Rc<super::jit::Code>),
            Compile,
            Nothing,
        }
        let act = match &mut self.jit.entry(key).or_insert_with(|| (f.clone(), JitSlot::Profiling(0))).1 {
            JitSlot::Compiled(c) => Act::Run(c.clone()),
            JitSlot::Bailed => Act::Nothing,
            JitSlot::Profiling(c) => {
                *c += 1;
                if *c >= JIT_THRESHOLD {
                    Act::Compile
                } else {
                    Act::Nothing
                }
            }
        };
        let code = match act {
            Act::Run(c) => Some(c),
            Act::Nothing => None,
            Act::Compile => {
                let compiled = super::jit::compile(f).map(Rc::new);
                let slot = match &compiled {
                    Some(c) => JitSlot::Compiled(c.clone()),
                    None => JitSlot::Bailed,
                };
                self.jit.insert(key, (f.clone(), slot));
                compiled
            }
        };
        let code = code?;
        if args.len() < code.nparams || !args.iter().take(code.nparams).all(|a| matches!(a, Val::Num(_))) {
            return None; // deopt: wrong arity or non-numeric arg
        }
        let fargs: Vec<f64> = (0..code.nparams).map(|i| args[i].as_num()).collect();
        Some(Val::Num(code.run(&fargs)))
    }

    pub fn call(&mut self, func: Val, this: Val, args: Vec<Val>) -> Result<Val, Val> {
        match func {
            Val::Func(f, captured) => {
                // Profile-guided JIT: numeric, call-free hot functions run as
                // native AArch64. We deopt to the interpreter for non-numeric
                // args or functions outside the compilable subset.
                if let Some(r) = self.try_jit(&f, &args) {
                    return Ok(r);
                }
                let inner = new_scope(Some(captured));
                if !f.arrow {
                    inner.borrow_mut().vars.insert("this".into(), this);
                    inner.borrow_mut().vars.insert("arguments".into(), Val::array(args.clone()));
                }
                // bind params
                for (i, p) in f.params.iter().enumerate() {
                    match p {
                        Pat::Array(items, rest) if items.is_empty() && rest.is_some() => {
                            // rest param
                            let tail: Vec<Val> = args.iter().skip(i).cloned().collect();
                            inner.borrow_mut().vars.insert(rest.clone().unwrap(), Val::array(tail));
                        }
                        _ => {
                            let v = args.get(i).cloned().unwrap_or(Val::Undef);
                            self.bind_pat(p, v, &inner)?;
                        }
                    }
                }
                let result = if let Some(eb) = &f.expr_body {
                    self.eval(eb, &inner)
                } else {
                    match self.exec_block(&f.body, &inner) {
                        Ok(Flow::Return(v)) => Ok(v),
                        Ok(_) => Ok(Val::Undef),
                        Err(e) => Err(e),
                    }
                };
                // async functions resolve to a Promise (and capture a throw as a
                // rejection) so `await f()` / `f().then()` work in our sync model.
                if f.is_async {
                    Ok(match result {
                        Ok(v) if Self::is_promise(&v) => v,
                        Ok(v) => self.make_promise("fulfilled", v),
                        Err(e) => self.make_promise("rejected", e),
                    })
                } else {
                    result
                }
            }
            Val::Native(Native::Global(name)) => self.call_global(&name, args),
            Val::Native(Native::Method(recv, name)) => {
                if let Some(r) = self.builtin_method(&recv, &name, &args)? {
                    Ok(r)
                } else {
                    Ok(Val::Undef)
                }
            }
            _ => Ok(Val::Undef),
        }
    }

    // ---- classes -----------------------------------------------------------

    /// Evaluate a class expression into a class object: a Val::Object carrying
    /// `__class`, `__ctor`, `__methods`, `__getters`, `__setters`, `__fields`,
    /// `__parent`, plus static members as direct keys.
    fn eval_class(&mut self, c: &Class, scope: &Scope) -> Result<Val, Val> {
        let parent = match &c.parent {
            Some(e) => self.eval(e, scope)?,
            None => Val::Null,
        };
        let mut obj = Obj::new();
        let name = c.name.clone().unwrap_or_else(|| String::from("Class"));
        obj.insert("__class".into(), Val::str(name));
        obj.insert("__parent".into(), parent);
        if let Some(ctor) = &c.ctor {
            obj.insert("__ctor".into(), Val::Func(rc_func(ctor), scope.clone()));
        }
        let mut methods = Obj::new();
        let mut getters = Obj::new();
        let mut setters = Obj::new();
        let mut fields: Vec<Val> = Vec::new();
        for (mname, f, is_static, kind) in &c.methods {
            let fv = Val::Func(rc_func(f), scope.clone());
            if *kind == "field" {
                // "@field:x" — field initializer; statics run now, instance later
                if *is_static {
                    let _ = self.call(fv, Val::object(obj.clone()), Vec::new());
                } else {
                    fields.push(fv);
                }
                continue;
            }
            if *is_static {
                obj.insert(mname.clone(), fv);
            } else if *kind == "get" {
                getters.insert(mname.clone(), fv);
            } else if *kind == "set" {
                setters.insert(mname.clone(), fv);
            } else {
                methods.insert(mname.clone(), fv);
            }
        }
        obj.insert("__methods".into(), Val::object(methods));
        obj.insert("__getters".into(), Val::object(getters));
        obj.insert("__setters".into(), Val::object(setters));
        obj.insert("__fields".into(), Val::array(fields));
        Ok(Val::object(obj))
    }

    /// The superclass visible from the current scope: the `__superclass` bound
    /// in a constructor, or (inside an instance method) the parent of the
    /// instance's own class — `this.__classref.__parent`.
    fn current_super(&self, scope: &Scope) -> Val {
        if let Some(sc) = self.lookup(scope, "__superclass") {
            if !matches!(sc, Val::Undef) {
                return sc;
            }
        }
        if let Some(Val::Object(inst)) = self.lookup(scope, "this") {
            if let Some(Val::Object(cls)) = inst.borrow().get("__classref") {
                return cls.borrow().get("__parent").cloned().unwrap_or(Val::Null);
            }
        }
        Val::Null
    }

    /// Look up an instance method `name` walking a class's parent chain.
    fn class_lookup_method(&self, cls: &Val, name: &str) -> Option<Val> {
        let mut cur = cls.clone();
        loop {
            let Val::Object(m) = &cur else { return None };
            let b = m.borrow();
            if let Some(Val::Object(methods)) = b.get("__methods") {
                if let Some(f) = methods.borrow().get(name) {
                    return Some(f.clone());
                }
            }
            let parent = b.get("__parent").cloned().unwrap_or(Val::Null);
            drop(b);
            cur = parent;
            if matches!(cur, Val::Null | Val::Undef) {
                return None;
            }
        }
    }

    /// Is `cls` a class object (has __methods)?
    fn is_class(v: &Val) -> bool {
        matches!(v, Val::Object(m) if m.borrow().contains_key("__methods"))
    }

    /// `obj instanceof ctor`: walk the instance's class chain comparing class
    /// names, or match a builtin constructor against the value's shape.
    fn instance_of(&self, obj: &Val, ctor: &Val) -> bool {
        // builtin: Array / Object / Map / Set / Promise / Error
        if let Val::Native(Native::Global(name)) = ctor {
            return match &**name {
                "Array" => matches!(obj, Val::Array(_)),
                "Object" => matches!(obj, Val::Object(_) | Val::Array(_)),
                "Map" | "WeakMap" => matches!(obj, Val::Object(m) if m.borrow().contains_key("__map")),
                "Set" | "WeakSet" => matches!(obj, Val::Object(m) if m.borrow().contains_key("__set")),
                "Promise" => Self::is_promise(obj),
                "Error" | "TypeError" | "RangeError" => matches!(obj, Val::Object(m) if m.borrow().contains_key("stack")),
                "Function" => matches!(obj, Val::Func(..) | Val::Native(_)),
                _ => false,
            };
        }
        // class instance: compare the instance's class chain to `ctor`'s name.
        let target = if let Val::Object(m) = ctor {
            m.borrow().get("__class").map(|v| v.to_str())
        } else {
            None
        };
        let Some(target) = target else { return false };
        let mut cur = if let Val::Object(m) = obj {
            m.borrow().get("__classref").cloned()
        } else {
            None
        };
        while let Some(Val::Object(m)) = cur {
            if m.borrow().get("__class").map(|v| v.to_str()).as_deref() == Some(target.as_str()) {
                return true;
            }
            cur = m.borrow().get("__parent").cloned();
        }
        false
    }

    /// Construct an instance, by class object or builtin constructor name.
    fn construct(&mut self, ctor: Val, name: &str, args: Vec<Val>) -> Result<Val, Val> {
        // Built-in constructors recognised by name.
        match name {
            "Promise" => return self.construct_promise(args),
            "Map" | "WeakMap" => return self.construct_map(args),
            "Set" | "WeakSet" => return self.construct_set(args),
            "WebSocket" => return self.construct_websocket(args),
            "Array" => return Ok(self.call_global("Array", args)?),
            "Error" | "TypeError" | "RangeError" | "SyntaxError" | "ReferenceError" => {
                let mut o = Obj::new();
                o.insert("message".into(), args.first().cloned().unwrap_or(Val::str("")));
                o.insert("name".into(), Val::str(name.to_string()));
                o.insert("stack".into(), Val::str(""));
                return Ok(Val::object(o));
            }
            "Date" => {
                let mut o = Obj::new();
                o.insert("__date".into(), Val::Num(0.0));
                return Ok(Val::object(o));
            }
            // Typed arrays — modelled as plain numeric arrays. `new Float32Array(n)`
            // makes a zero-filled length-n array; `new Float32Array([…])` copies.
            "Float32Array" | "Float64Array" | "Int32Array" | "Uint32Array"
            | "Int16Array" | "Uint16Array" | "Int8Array" | "Uint8Array" | "Uint8ClampedArray" => {
                return Ok(match args.into_iter().next() {
                    Some(Val::Array(a)) => Val::array(a.borrow().clone()),
                    Some(Val::Num(n)) => Val::array(alloc::vec![Val::Num(0.0); n.max(0.0) as usize]),
                    _ => Val::array(Vec::new()),
                });
            }
            "Object" => return Ok(args.into_iter().next().unwrap_or_else(|| Val::object(Obj::new()))),
            "RegExp" => {
                let mut o = Obj::new();
                o.insert("source".into(), args.first().cloned().unwrap_or(Val::str("")));
                o.insert("__regex".into(), Val::Bool(true));
                return Ok(Val::object(o));
            }
            // DOM events: new Event(type, init) / new CustomEvent(type, init).
            "Event" | "CustomEvent" => {
                let mut o = Obj::new();
                o.insert("type".into(), args.first().cloned().unwrap_or(Val::str("")));
                o.insert("bubbles".into(), Val::Bool(false));
                o.insert("cancelable".into(), Val::Bool(false));
                o.insert("defaultPrevented".into(), Val::Bool(false));
                if let Some(Val::Object(init)) = args.get(1) {
                    let ib = init.borrow();
                    if let Some(b) = ib.get("bubbles") { o.insert("bubbles".into(), b.clone()); }
                    if let Some(d) = ib.get("detail") { o.insert("detail".into(), d.clone()); }
                }
                o.insert("preventDefault".into(), Val::Native(Native::Global(Rc::from("noop"))));
                o.insert("stopPropagation".into(), Val::Native(Native::Global(Rc::from("noop"))));
                return Ok(Val::object(o));
            }
            // MutationObserver(cb): records the callback; observe/disconnect/takeRecords.
            "MutationObserver" => {
                let mut o = Obj::new();
                o.insert("__mutobs".into(), args.first().cloned().unwrap_or(Val::Undef));
                o.insert("observe".into(), Val::Native(Native::Global(Rc::from("noop"))));
                o.insert("disconnect".into(), Val::Native(Native::Global(Rc::from("noop"))));
                o.insert("takeRecords".into(), Val::array(Vec::new()));
                return Ok(Val::object(o));
            }
            // MessageChannel: two LINKED ports — port.postMessage(data) delivers
            // a 'message' event to the *peer* port's onmessage on the next tick.
            // React 18's scheduler drives its entire work loop through this
            // (port1.onmessage = flushWork; port2.postMessage(null) schedules it),
            // so the peer link is what makes the reconciler actually run + commit.
            "MessageChannel" => {
                let mk_port = || {
                    let mut p = Obj::new();
                    p.insert("__port".into(), Val::Bool(true));
                    p.insert("onmessage".into(), Val::Null);
                    p.insert("close".into(), Val::Native(Native::Global(Rc::from("noop"))));
                    p.insert("start".into(), Val::Native(Native::Global(Rc::from("noop"))));
                    Rc::new(RefCell::new(p))
                };
                let p1 = mk_port();
                let p2 = mk_port();
                // cross-link the peers so postMessage can reach the other side.
                p1.borrow_mut().insert("__peer".into(), Val::Object(p2.clone()));
                p2.borrow_mut().insert("__peer".into(), Val::Object(p1.clone()));
                let mut o = Obj::new();
                o.insert("port1".into(), Val::Object(p1));
                o.insert("port2".into(), Val::Object(p2));
                return Ok(Val::object(o));
            }
            "AbortController" => {
                let mut sig = Obj::new();
                sig.insert("aborted".into(), Val::Bool(false));
                sig.insert("addEventListener".into(), Val::Native(Native::Global(Rc::from("noop"))));
                let mut o = Obj::new();
                o.insert("signal".into(), Val::object(sig));
                o.insert("abort".into(), Val::Native(Native::Global(Rc::from("noop"))));
                return Ok(Val::object(o));
            }
            // M42 step 18: V8-parity web platform constructors.
            "URL" => {
                let u = args.first().map(|v| v.to_str()).unwrap_or_default();
                return Ok(parse_url(&u));
            }
            "URLSearchParams" => {
                return Ok(make_url_search_params(&args.first().map(|v| v.to_str()).unwrap_or_default()));
            }
            "TextEncoder" => {
                let mut o = Obj::new();
                o.insert("encoding".into(), Val::str("utf-8"));
                o.insert("encode".into(), Val::Native(Native::Global(Rc::from("TextEncoder.encode"))));
                return Ok(Val::object(o));
            }
            "TextDecoder" => {
                let mut o = Obj::new();
                o.insert("encoding".into(), Val::str("utf-8"));
                o.insert("decode".into(), Val::Native(Native::Global(Rc::from("TextDecoder.decode"))));
                return Ok(Val::object(o));
            }
            "FormData" => {
                let mut o = Obj::new();
                o.insert("__formdata".into(), Val::object(Obj::new()));
                for m in ["append", "set"] { o.insert(m.into(), Val::Native(Native::Global(Rc::from("FormData.append")))); }
                o.insert("get".into(), Val::Native(Native::Global(Rc::from("FormData.get"))));
                o.insert("has".into(), Val::Native(Native::Global(Rc::from("FormData.has"))));
                return Ok(Val::object(o));
            }
            "Blob" => {
                let mut o = Obj::new();
                // join the parts array for a size + text()
                let text: String = match args.first() {
                    Some(Val::Array(a)) => a.borrow().iter().map(|v| v.to_str()).collect(),
                    _ => String::new(),
                };
                o.insert("size".into(), Val::Num(text.len() as f64));
                o.insert("type".into(), Val::str(""));
                o.insert("__blob".into(), Val::str(text.clone()));
                return Ok(Val::object(o));
            }
            "WeakRef" => {
                let mut o = Obj::new();
                o.insert("__ref".into(), args.first().cloned().unwrap_or(Val::Undef));
                o.insert("deref".into(), Val::Native(Native::Global(Rc::from("WeakRef.deref"))));
                return Ok(Val::object(o));
            }
            _ => {}
        }
        if Self::is_class(&ctor) {
            return self.instantiate_class(&ctor, args);
        }
        // Pre-ES6 constructor function: run with this = fresh object whose
        // hidden __proto__ links to the constructor's prototype, so instance
        // method lookups inherit `Ctor.prototype.method`.
        if let Val::Func(rc, _) = &ctor {
            let proto = self.func_bag(rc).borrow().get("prototype").cloned();
            let mut m = Obj::new();
            if let Some(p) = proto {
                m.insert("__proto__".into(), p);
            }
            let obj = Val::object(m);
            let r = self.call(ctor.clone(), obj.clone(), args)?;
            // ctor may return an object; otherwise the new object.
            return Ok(if matches!(r, Val::Object(_)) { r } else { obj });
        }
        Ok(Val::object(Obj::new()))
    }

    fn instantiate_class(&mut self, cls: &Val, args: Vec<Val>) -> Result<Val, Val> {
        // Build the instance: copy methods from the whole chain (parent first so
        // subclasses override), record the class name + superclass for super.*.
        let mut inst = Obj::new();
        let mut chain: Vec<Val> = Vec::new();
        let mut cur = cls.clone();
        while let Val::Object(m) = &cur {
            chain.push(cur.clone());
            let p = m.borrow().get("__parent").cloned().unwrap_or(Val::Null);
            if matches!(p, Val::Null | Val::Undef) {
                break;
            }
            cur = p;
        }
        let class_name = if let Val::Object(m) = cls {
            m.borrow().get("__class").map(|v| v.to_str()).unwrap_or_default()
        } else {
            String::new()
        };
        inst.insert("__class".into(), Val::str(class_name));
        inst.insert("__classref".into(), cls.clone());
        // parent first
        for c in chain.iter().rev() {
            if let Val::Object(m) = c {
                if let Some(Val::Object(methods)) = m.borrow().get("__methods") {
                    for (k, v) in methods.borrow().iter() {
                        inst.insert(k.clone(), v.clone());
                    }
                }
                if let Some(Val::Object(g)) = m.borrow().get("__getters") {
                    for (k, v) in g.borrow().iter() {
                        inst.insert(alloc::format!("__get:{k}"), v.clone());
                    }
                }
                if let Some(Val::Object(s)) = m.borrow().get("__setters") {
                    for (k, v) in s.borrow().iter() {
                        inst.insert(alloc::format!("__set:{k}"), v.clone());
                    }
                }
            }
        }
        let obj = Val::object(inst);
        // Field initializers (parent first), then the constructor chain.
        for c in chain.iter().rev() {
            if let Val::Object(m) = c {
                if let Some(Val::Array(fields)) = m.borrow().get("__fields") {
                    for f in fields.borrow().iter() {
                        self.call(f.clone(), obj.clone(), Vec::new())?;
                    }
                }
            }
        }
        self.run_ctor_chain(cls, obj.clone(), args)?;
        Ok(obj)
    }

    /// Run a class's constructor (or, lacking one, implicitly forward to the
    /// parent) with `this` already created. super(...) inside re-enters here.
    fn run_ctor_chain(&mut self, cls: &Val, this: Val, args: Vec<Val>) -> Result<(), Val> {
        let Val::Object(m) = cls else { return Ok(()) };
        let ctor = m.borrow().get("__ctor").cloned();
        let parent = m.borrow().get("__parent").cloned().unwrap_or(Val::Null);
        match ctor {
            Some(Val::Func(f, captured)) => {
                let inner = new_scope(Some(captured));
                inner.borrow_mut().vars.insert("this".into(), this);
                inner.borrow_mut().vars.insert("__superclass".into(), parent);
                inner.borrow_mut().vars.insert("arguments".into(), Val::array(args.clone()));
                for (i, p) in f.params.iter().enumerate() {
                    match p {
                        Pat::Array(items, rest) if items.is_empty() && rest.is_some() => {
                            let tail: Vec<Val> = args.iter().skip(i).cloned().collect();
                            inner.borrow_mut().vars.insert(rest.clone().unwrap(), Val::array(tail));
                        }
                        _ => {
                            let v = args.get(i).cloned().unwrap_or(Val::Undef);
                            self.bind_pat(p, v, &inner)?;
                        }
                    }
                }
                self.exec_block(&f.body, &inner)?;
                Ok(())
            }
            _ => {
                // No own constructor: implicitly super(...args).
                if Self::is_class(&parent) {
                    self.run_ctor_chain(&parent, this, args)?;
                }
                Ok(())
            }
        }
    }

    // ---- promises (synchronous model) --------------------------------------

    fn is_promise(v: &Val) -> bool {
        matches!(v, Val::Object(m) if matches!(m.borrow().get("__promise"), Some(Val::Bool(true))))
    }

    fn make_promise(&self, state: &str, value: Val) -> Val {
        let mut o = Obj::new();
        o.insert("__promise".into(), Val::Bool(true));
        o.insert("__state".into(), Val::str(state.to_string()));
        o.insert("__value".into(), value);
        Val::object(o)
    }

    /// Unwrap a resolved promise (or pass through a plain value); reject throws.
    fn await_val(&mut self, v: Val) -> Result<Val, Val> {
        self.drain_deferred();
        if let Val::Object(m) = &v {
            let b = m.borrow();
            if matches!(b.get("__promise"), Some(Val::Bool(true))) {
                let state = b.get("__state").map(|s| s.to_str()).unwrap_or_default();
                let val = b.get("__value").cloned().unwrap_or(Val::Undef);
                drop(b);
                if state == "rejected" {
                    return Err(val);
                }
                // a promise of a promise: unwrap again
                return self.await_val(val);
            }
        }
        Ok(v)
    }

    fn construct_promise(&mut self, args: Vec<Val>) -> Result<Val, Val> {
        // new Promise((resolve, reject) => ...) — run the executor synchronously.
        // resolve/reject are native closures (id into `resolvers`) that record
        // the outcome into a shared cell, which we read back after the executor.
        let cell = Rc::new(RefCell::new((String::from("pending"), Val::Undef)));
        let id = self.resolvers.len();
        self.resolvers.push(cell.clone());
        let resolve = Val::Native(Native::Global(Rc::from(alloc::format!("__resolve:{id}").as_str())));
        let reject = Val::Native(Native::Global(Rc::from(alloc::format!("__reject:{id}").as_str())));
        if let Some(exec) = args.first().cloned() {
            if let Err(e) = self.call(exec, Val::Undef, alloc::vec![resolve, reject]) {
                return Ok(self.make_promise("rejected", e));
            }
        }
        let (state, value) = cell.borrow().clone();
        if state == "pending" {
            Ok(self.make_promise("fulfilled", Val::Undef))
        } else {
            Ok(self.make_promise(&state, value))
        }
    }

    fn promise_method(&mut self, o: &Rc<RefCell<Obj>>, name: &str, args: &[Val]) -> Result<Val, Val> {
        let state = o.borrow().get("__state").map(|s| s.to_str()).unwrap_or_default();
        let value = o.borrow().get("__value").cloned().unwrap_or(Val::Undef);
        match name {
            "then" => {
                let mut result = Val::object(o.borrow().clone());
                if state == "fulfilled" {
                    if let Some(cb) = args.first().cloned() {
                        let r = self.call(cb, Val::Undef, alloc::vec![value])?;
                        result = if Self::is_promise(&r) { r } else { self.make_promise("fulfilled", r) };
                    }
                } else if state == "rejected" {
                    if let Some(cb) = args.get(1).cloned() {
                        let r = self.call(cb, Val::Undef, alloc::vec![value])?;
                        result = if Self::is_promise(&r) { r } else { self.make_promise("fulfilled", r) };
                    }
                }
                Ok(result)
            }
            "catch" => {
                if state == "rejected" {
                    if let Some(cb) = args.first().cloned() {
                        let r = self.call(cb, Val::Undef, alloc::vec![value])?;
                        return Ok(if Self::is_promise(&r) { r } else { self.make_promise("fulfilled", r) });
                    }
                }
                Ok(Val::object(o.borrow().clone()))
            }
            "finally" => {
                if let Some(cb) = args.first().cloned() {
                    self.call(cb, Val::Undef, Vec::new())?;
                }
                Ok(Val::object(o.borrow().clone()))
            }
            _ => Ok(Val::Undef),
        }
    }

    // ---- Map / Set ---------------------------------------------------------

    fn construct_map(&mut self, args: Vec<Val>) -> Result<Val, Val> {
        let mut o = Obj::new();
        let mut entries: Vec<Val> = Vec::new();
        if let Some(it) = args.first() {
            for pair in self.to_vec(it) {
                entries.push(pair);
            }
        }
        o.insert("__map".into(), Val::array(entries));
        Ok(Val::object(o))
    }

    fn construct_set(&mut self, args: Vec<Val>) -> Result<Val, Val> {
        let mut o = Obj::new();
        let mut vals: Vec<Val> = Vec::new();
        if let Some(it) = args.first() {
            for v in self.to_vec(it) {
                if !vals.iter().any(|x| strict_eq(x, &v)) {
                    vals.push(v);
                }
            }
        }
        o.insert("__set".into(), Val::array(vals));
        Ok(Val::object(o))
    }

    fn construct_websocket(&mut self, args: Vec<Val>) -> Result<Val, Val> {
        let url = args.first().map(|v| v.to_str()).unwrap_or_default();
        let mut o = Obj::new();
        o.insert("url".into(), Val::str(url.clone()));
        match crate::browser::js_ws_open(&url) {
            Some(id) => {
                o.insert("__ws".into(), Val::Num(id as f64));
                o.insert("readyState".into(), Val::Num(1.0)); // OPEN
                let obj = Val::object(o);
                // Fire onopen asynchronously (after the script sets the handler).
                self.deferred.push((Val::Native(Native::Global(Rc::from("__ws_open"))), alloc::vec![obj.clone()]));
                Ok(obj)
            }
            None => {
                o.insert("readyState".into(), Val::Num(3.0)); // CLOSED
                let obj = Val::object(o);
                self.deferred.push((Val::Native(Native::Global(Rc::from("__ws_error"))), alloc::vec![obj.clone()]));
                Ok(obj)
            }
        }
    }

    /// Methods on a WebSocket object (`__ws` present): send / close.
    fn ws_method(&mut self, o: &Rc<RefCell<Obj>>, name: &str, args: &[Val]) -> Result<Val, Val> {
        let id = o.borrow().get("__ws").map(|v| v.as_num() as usize);
        let Some(id) = id else { return Ok(Val::Undef) };
        match name {
            "send" => {
                let msg = args.first().map(|v| v.to_str()).unwrap_or_default();
                if let Some(reply) = crate::browser::js_ws_send_recv(id, &msg) {
                    // Deliver the reply to onmessage (event { data, type }).
                    let onmsg = o.borrow().get("onmessage").cloned();
                    if let Some(h) = onmsg {
                        let mut ev = Obj::new();
                        ev.insert("data".into(), Val::str(reply));
                        ev.insert("type".into(), Val::str("message"));
                        let this = Val::object(o.borrow().clone());
                        self.call(h, this, alloc::vec![Val::object(ev)])?;
                    }
                }
                Ok(Val::Undef)
            }
            "close" => {
                crate::browser::js_ws_close(id);
                o.borrow_mut().insert("readyState".into(), Val::Num(3.0));
                let onclose = o.borrow().get("onclose").cloned();
                if let Some(h) = onclose {
                    let mut ev = Obj::new();
                    ev.insert("type".into(), Val::str("close"));
                    let this = Val::object(o.borrow().clone());
                    self.call(h, this, alloc::vec![Val::object(ev)])?;
                }
                Ok(Val::Undef)
            }
            "addEventListener" => {
                // map ('message'|'open'|'close', handler) onto on* properties
                let ev = args.first().map(|v| v.to_str()).unwrap_or_default();
                if let Some(h) = args.get(1) {
                    o.borrow_mut().insert(alloc::format!("on{ev}"), h.clone());
                }
                Ok(Val::Undef)
            }
            _ => Ok(Val::Undef),
        }
    }

    fn map_entries(&self, o: &Rc<RefCell<Obj>>) -> Rc<RefCell<Vec<Val>>> {
        if let Some(Val::Array(a)) = o.borrow().get("__map") {
            return a.clone();
        }
        Rc::new(RefCell::new(Vec::new()))
    }

    fn map_method(&mut self, o: &Rc<RefCell<Obj>>, name: &str, args: &[Val]) -> Result<Val, Val> {
        let entries = self.map_entries(o);
        let a0 = args.first().cloned().unwrap_or(Val::Undef);
        let find = |k: &Val| entries.borrow().iter().position(|e| {
            matches!(e, Val::Array(p) if p.borrow().first().map(|x| strict_eq(x, k)).unwrap_or(false))
        });
        Ok(match name {
            "get" => find(&a0)
                .and_then(|i| match &entries.borrow()[i] {
                    Val::Array(p) => p.borrow().get(1).cloned(),
                    _ => None,
                })
                .unwrap_or(Val::Undef),
            "set" => {
                let v1 = args.get(1).cloned().unwrap_or(Val::Undef);
                if let Some(i) = find(&a0) {
                    if let Val::Array(p) = &entries.borrow()[i] {
                        let mut pb = p.borrow_mut();
                        if pb.len() < 2 {
                            pb.push(v1);
                        } else {
                            pb[1] = v1;
                        }
                    }
                } else {
                    entries.borrow_mut().push(Val::array(alloc::vec![a0, v1]));
                }
                Val::object(o.borrow().clone())
            }
            "has" => Val::Bool(find(&a0).is_some()),
            "delete" => {
                if let Some(i) = find(&a0) {
                    entries.borrow_mut().remove(i);
                    Val::Bool(true)
                } else {
                    Val::Bool(false)
                }
            }
            "clear" => {
                entries.borrow_mut().clear();
                Val::Undef
            }
            "forEach" => {
                let snap = entries.borrow().clone();
                for e in snap {
                    if let Val::Array(p) = e {
                        let k = p.borrow().first().cloned().unwrap_or(Val::Undef);
                        let v = p.borrow().get(1).cloned().unwrap_or(Val::Undef);
                        self.call(a0.clone(), Val::Undef, alloc::vec![v, k])?;
                    }
                }
                Val::Undef
            }
            "keys" => Val::array(entries.borrow().iter().filter_map(|e| match e {
                Val::Array(p) => p.borrow().first().cloned(),
                _ => None,
            }).collect()),
            "values" => Val::array(entries.borrow().iter().filter_map(|e| match e {
                Val::Array(p) => p.borrow().get(1).cloned(),
                _ => None,
            }).collect()),
            "entries" => Val::array(entries.borrow().clone()),
            _ => Val::Undef,
        })
    }

    fn set_values(&self, o: &Rc<RefCell<Obj>>) -> Rc<RefCell<Vec<Val>>> {
        if let Some(Val::Array(a)) = o.borrow().get("__set") {
            return a.clone();
        }
        Rc::new(RefCell::new(Vec::new()))
    }

    fn set_method(&mut self, o: &Rc<RefCell<Obj>>, name: &str, args: &[Val]) -> Result<Val, Val> {
        let vals = self.set_values(o);
        let a0 = args.first().cloned().unwrap_or(Val::Undef);
        Ok(match name {
            "add" => {
                if !vals.borrow().iter().any(|x| strict_eq(x, &a0)) {
                    vals.borrow_mut().push(a0);
                }
                Val::object(o.borrow().clone())
            }
            "has" => Val::Bool(vals.borrow().iter().any(|x| strict_eq(x, &a0))),
            "delete" => {
                let pos = vals.borrow().iter().position(|x| strict_eq(x, &a0));
                if let Some(i) = pos {
                    vals.borrow_mut().remove(i);
                    Val::Bool(true)
                } else {
                    Val::Bool(false)
                }
            }
            "clear" => {
                vals.borrow_mut().clear();
                Val::Undef
            }
            "forEach" => {
                let snap = vals.borrow().clone();
                for v in snap {
                    self.call(a0.clone(), Val::Undef, alloc::vec![v.clone(), v])?;
                }
                Val::Undef
            }
            "values" | "keys" => Val::array(vals.borrow().clone()),
            _ => Val::Undef,
        })
    }

    /// Dispatch methods on special objects (promise / map / set / Response).
    /// Returns None for plain objects so user function props are used instead.
    fn object_method(&mut self, o: Rc<RefCell<Obj>>, name: &str, args: &[Val]) -> Result<Option<Val>, Val> {
        let kind = {
            let b = o.borrow();
            if b.contains_key("__promise") {
                1
            } else if b.contains_key("__map") {
                2
            } else if b.contains_key("__set") {
                3
            } else if b.contains_key("__body") {
                4
            } else if b.contains_key("__ws") {
                5
            } else if b.contains_key("__usp") {
                6
            } else if b.contains_key("__formdata") {
                7
            } else if b.contains_key("__blob") {
                8
            } else if b.contains_key("__port") {
                9
            } else {
                0
            }
        };
        match kind {
            1 => Ok(Some(self.promise_method(&o, name, args)?)),
            2 => Ok(Some(self.map_method(&o, name, args)?)),
            3 => Ok(Some(self.set_method(&o, name, args)?)),
            5 => Ok(Some(self.ws_method(&o, name, args)?)),
            6 => {
                // URLSearchParams: pairs in __usp = [[k,v],...].
                let key = args.first().map(|v| v.to_str()).unwrap_or_default();
                let pairs: Vec<(String, String)> = match o.borrow().get("__usp") {
                    Some(Val::Array(a)) => a.borrow().iter().filter_map(|p| match p {
                        Val::Array(kv) => { let kv = kv.borrow(); Some((kv.first()?.to_str(), kv.get(1)?.to_str())) }
                        _ => None,
                    }).collect(),
                    _ => Vec::new(),
                };
                Ok(Some(match name {
                    "get" => pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| Val::str(v.clone())).unwrap_or(Val::Null),
                    "getAll" => Val::array(pairs.iter().filter(|(k, _)| *k == key).map(|(_, v)| Val::str(v.clone())).collect()),
                    "has" => Val::Bool(pairs.iter().any(|(k, _)| *k == key)),
                    "keys" => Val::array(pairs.iter().map(|(k, _)| Val::str(k.clone())).collect()),
                    "toString" => Val::str(pairs.iter().map(|(k, v)| alloc::format!("{k}={v}")).collect::<Vec<_>>().join("&")),
                    "append" | "set" => {
                        if let Some(Val::Array(a)) = o.borrow().get("__usp") {
                            a.borrow_mut().push(Val::array(alloc::vec![Val::str(key), args.get(1).cloned().unwrap_or(Val::str(""))]));
                        }
                        Val::Undef
                    }
                    _ => Val::Undef,
                }))
            }
            7 => {
                // FormData: __formdata is an object map.
                let key = args.first().map(|v| v.to_str()).unwrap_or_default();
                Ok(Some(match name {
                    "append" | "set" => {
                        if let Some(Val::Object(m)) = o.borrow().get("__formdata") {
                            m.borrow_mut().insert(key, args.get(1).cloned().unwrap_or(Val::str("")));
                        }
                        Val::Undef
                    }
                    "get" => match o.borrow().get("__formdata") {
                        Some(Val::Object(m)) => m.borrow().get(&key).cloned().unwrap_or(Val::Null),
                        _ => Val::Null,
                    },
                    "has" => Val::Bool(matches!(o.borrow().get("__formdata"), Some(Val::Object(m)) if m.borrow().contains_key(&key))),
                    _ => Val::Undef,
                }))
            }
            8 => {
                // Blob: text() returns the joined content as a resolved promise.
                let body = o.borrow().get("__blob").map(|v| v.to_str()).unwrap_or_default();
                Ok(Some(match name {
                    "text" => self.make_promise("fulfilled", Val::str(body)),
                    _ => Val::Undef,
                }))
            }
            9 => {
                // MessageChannel port: postMessage(data) -> defer the PEER port's
                // onmessage({data}). This is what drives React's scheduler loop.
                Ok(Some(match name {
                    "postMessage" => {
                        let peer = o.borrow().get("__peer").cloned();
                        if let Some(Val::Object(peer)) = peer {
                            let onmsg = peer.borrow().get("onmessage").cloned();
                            if let Some(handler) = onmsg {
                                if !matches!(handler, Val::Null | Val::Undef) {
                                    let mut ev = Obj::new();
                                    ev.insert("data".into(), args.first().cloned().unwrap_or(Val::Undef));
                                    ev.insert("type".into(), Val::str("message"));
                                    self.deferred.push((handler, alloc::vec![Val::object(ev)]));
                                }
                            }
                        }
                        Val::Undef
                    }
                    "start" | "close" => Val::Undef,
                    _ => Val::Undef,
                }))
            }
            4 => {
                // fetch() Response
                let body = o.borrow().get("__body").map(|v| v.to_str()).unwrap_or_default();
                match name {
                    "text" => Ok(Some(self.make_promise("fulfilled", Val::str(body)))),
                    "json" => {
                        let v = json_parse(&body);
                        Ok(Some(self.make_promise("fulfilled", v)))
                    }
                    _ => Ok(Some(Val::Undef)),
                }
            }
            _ => Ok(None),
        }
    }

    // ---- assignment targets ------------------------------------------------

    fn assign_to(&mut self, target: &Expr, v: Val, scope: &Scope) -> Result<(), Val> {
        match target {
            Expr::Ident(n) => {
                self.set_var(scope, n, v);
                Ok(())
            }
            Expr::Member(obj, prop, _) => {
                let o = self.eval(obj, scope)?;
                self.set_member(o, prop, v);
                Ok(())
            }
            Expr::Index(obj, idx) => {
                let o = self.eval(obj, scope)?;
                let i = self.eval(idx, scope)?;
                self.set_index(o, i, v);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn lookup(&self, scope: &Scope, name: &str) -> Option<Val> {
        let mut cur = Some(scope.clone());
        while let Some(s) = cur {
            if let Some(v) = s.borrow().vars.get(name) {
                return Some(v.clone());
            }
            cur = s.borrow().parent.clone();
        }
        None
    }

    fn set_var(&self, scope: &Scope, name: &str, v: Val) {
        let mut cur = Some(scope.clone());
        while let Some(s) = cur {
            if s.borrow().vars.contains_key(name) {
                s.borrow_mut().vars.insert(name.into(), v);
                return;
            }
            cur = s.borrow().parent.clone();
        }
        // implicit global
        self.global.borrow_mut().vars.insert(name.into(), v);
    }

    // ---- member get/set & index --------------------------------------------

    fn get_index(&mut self, o: Val, i: Val) -> Result<Val, Val> {
        match &o {
            Val::Array(a) => {
                let idx = i.as_num();
                if idx >= 0.0 {
                    Ok(a.borrow().get(idx as usize).cloned().unwrap_or(Val::Undef))
                } else {
                    Ok(Val::Undef)
                }
            }
            Val::Str(s) => {
                let idx = i.as_num() as usize;
                Ok(s.chars().nth(idx).map(|c| Val::str(c.to_string())).unwrap_or(Val::Undef))
            }
            Val::Object(map) => Ok(map.borrow().get(&i.to_str()).cloned().unwrap_or(Val::Undef)),
            _ => self.get_member(o, &i.to_str()),
        }
    }

    fn set_index(&mut self, o: Val, i: Val, v: Val) {
        match o {
            Val::Array(a) => {
                let idx = i.as_num();
                if idx >= 0.0 {
                    let mut b = a.borrow_mut();
                    let idx = idx as usize;
                    if idx >= b.len() {
                        b.resize(idx + 1, Val::Undef);
                    }
                    b[idx] = v;
                }
            }
            Val::Object(map) => {
                map.borrow_mut().insert(i.to_str(), v);
            }
            other => self.set_member(other, &i.to_str(), v),
        }
    }

    /// The property bag for a function object (creating it, with an empty
    /// `prototype` object, on first access). Used for `.prototype` and statics.
    fn func_bag(&mut self, rc: &Rc<Func>) -> Rc<RefCell<Obj>> {
        let key = Rc::as_ptr(rc) as usize;
        if let Some((_, bag)) = self.func_props.get(&key) {
            return bag.clone();
        }
        let mut bag = Obj::new();
        bag.insert("prototype".into(), Val::object(Obj::new()));
        let bag = Rc::new(RefCell::new(bag));
        self.func_props.insert(key, (rc.clone(), bag.clone()));
        bag
    }

    fn get_member(&mut self, o: Val, prop: &str) -> Result<Val, Val> {
        match &o {
            // Function objects: `.prototype`, statics, and call/apply/bind.
            Val::Func(rc, _) => {
                if matches!(prop, "call" | "apply" | "bind") {
                    return Ok(Val::Native(Native::Method(Box::new(o.clone()), Rc::from(prop))));
                }
                let bag = self.func_bag(rc);
                let r = bag.borrow().get(prop).cloned();
                return Ok(r.unwrap_or(Val::Undef));
            }
            Val::Object(map) => {
                // size on Map/Set
                if prop == "size" {
                    let b = map.borrow();
                    if let Some(Val::Array(a)) = b.get("__map").or_else(|| b.get("__set")) {
                        return Ok(Val::Num(a.borrow().len() as f64));
                    }
                }
                // getter
                {
                    let gkey = alloc::format!("__get:{prop}");
                    let getter = map.borrow().get(&gkey).cloned();
                    if let Some(g) = getter {
                        return self.call(g, o.clone(), Vec::new());
                    }
                }
                if let Some(v) = map.borrow().get(prop) {
                    return Ok(v.clone());
                }
                // `constructor` on a class instance returns its class object.
                if prop == "constructor" {
                    if let Some(c) = map.borrow().get("__classref") {
                        return Ok(c.clone());
                    }
                }
                // Prototype chain: an instance built by `new Ctor()` carries a
                // hidden `__proto__` to Ctor.prototype — inherit its members.
                let proto = map.borrow().get("__proto__").cloned();
                if let Some(p @ Val::Object(_)) = proto {
                    let inherited = self.get_member(p, prop)?;
                    if !matches!(inherited, Val::Undef) {
                        return Ok(inherited);
                    }
                }
                // Special objects (Map/Set/Promise/Response/WebSocket) expose methods.
                if map.borrow().keys().any(|k| matches!(k.as_str(), "__map" | "__set" | "__promise" | "__body" | "__ws")) {
                    return Ok(Val::Native(Native::Method(Box::new(o.clone()), Rc::from(prop))));
                }
                Ok(Val::Undef)
            }
            Val::Array(a) => match prop {
                "length" => Ok(Val::Num(a.borrow().len() as f64)),
                _ => Ok(Val::Native(Native::Method(Box::new(o.clone()), Rc::from(prop)))),
            },
            Val::Str(s) => match prop {
                "length" => Ok(Val::Num(s.chars().count() as f64)),
                _ => Ok(Val::Native(Native::Method(Box::new(o.clone()), Rc::from(prop)))),
            },
            Val::Num(_) => Ok(Val::Native(Native::Method(Box::new(o.clone()), Rc::from(prop)))),
            Val::Node(idx) => Ok(self.node_member(*idx, prop)),
            Val::Host(h) => Ok(self.host_member(h.clone(), prop)),
            // Namespace/constructor objects: Object.keys, Array.from, Promise.all,
            // Number.isInteger, JSON.parse, ... resolve to a "Name.prop" native.
            Val::Native(Native::Global(name)) => {
                match (&**name, prop) {
                    ("Math", _) | ("Number", "MAX_SAFE_INTEGER") => {}
                    _ => {}
                }
                if prop == "prototype" {
                    return Ok(Val::object(Obj::new()));
                }
                Ok(Val::Native(Native::Global(Rc::from(alloc::format!("{name}.{prop}").as_str()))))
            }
            _ => Ok(Val::Undef),
        }
    }

    fn set_member(&mut self, o: Val, prop: &str, v: Val) {
        match &o {
            Val::Object(map) => {
                // setter?
                let skey = alloc::format!("__set:{prop}");
                let setter = map.borrow().get(&skey).cloned();
                if let Some(s) = setter {
                    let _ = self.call(s, o.clone(), alloc::vec![v]);
                    return;
                }
                map.borrow_mut().insert(prop.into(), v);
            }
            // Function statics / prototype replacement (`Ctor.prototype = {…}`).
            Val::Func(rc, _) => {
                let bag = self.func_bag(rc);
                bag.borrow_mut().insert(prop.into(), v);
            }
            Val::Node(idx) => self.set_node_member(*idx, prop, v),
            Val::Host(Host::Style(idx)) => {
                self.dom.set_style(*idx, prop, &v.to_str());
            }
            Val::Host(h @ (Host::LocalStorage | Host::SessionStorage)) => {
                let local = matches!(h, Host::LocalStorage);
                let origin = crate::browser::current_origin();
                crate::browser::storage_set(local, &origin, prop, &v.to_str());
            }
            Val::Host(Host::Canvas(n)) => {
                if let Some(c) = self.canvases.get_mut(*n) {
                    c.set_prop(prop, &v.to_str());
                }
            }
            Val::Host(Host::Location) => { /* navigation ignored */ }
            // Assigning a property to window/self/globalThis defines a real
            // global — this is how UMD bundles (React, etc.) expose themselves
            // (`global.React = {}`). The global object IS the window.
            Val::Host(Host::Window) => {
                self.global.borrow_mut().vars.insert(prop.into(), v);
            }
            _ => {}
        }
    }

    // ---- DOM node members --------------------------------------------------

    fn node_member(&self, idx: usize, prop: &str) -> Val {
        let n = &self.dom.nodes[idx];
        match prop {
            "id" => Val::str(n.attr("id").unwrap_or("").to_string()),
            "className" => Val::str(n.attr("class").unwrap_or("").to_string()),
            "tagName" => Val::str(n.tag.to_ascii_uppercase()),
            "nodeName" => Val::str(if n.is_text() { String::from("#text") } else { n.tag.to_ascii_uppercase() }),
            "nodeType" => Val::Num(self.dom.node_type(idx) as f64),
            "nodeValue" | "data" if n.is_text() => Val::str(n.text.clone()),
            "nodeValue" => Val::Null,
            "ownerDocument" => Val::Host(Host::Document),
            "namespaceURI" => Val::str("http://www.w3.org/1999/xhtml"),
            "isConnected" => Val::Bool(true),
            "innerHTML" => Val::str(self.dom.inner_html(idx)),
            "outerHTML" => Val::str(self.dom.inner_html(idx)),
            "textContent" | "innerText" => Val::str(self.dom.text_content(idx)),
            "value" => Val::str(n.attr("value").unwrap_or("").to_string()),
            "src" => Val::str(n.attr("src").unwrap_or("").to_string()),
            "href" => Val::str(n.attr("href").unwrap_or("").to_string()),
            "content" => Val::str(n.attr("content").unwrap_or("").to_string()),
            "title" => Val::str(n.attr("title").unwrap_or("").to_string()),
            "target" => Val::str(n.attr("target").unwrap_or("").to_string()),
            "type" => Val::str(n.attr("type").unwrap_or("").to_string()),
            "checked" => Val::Bool(n.attr("checked").is_some()),
            "hidden" => Val::Bool(n.attr("hidden").is_some()),
            "style" => Val::Host(Host::Style(idx)),
            "classList" => Val::Host(Host::ClassList(idx)),
            "dataset" => Val::Host(Host::Dataset(idx)),
            "parentNode" | "parentElement" => n.parent.map(Val::Node).unwrap_or(Val::Null),
            "children" | "childNodes" => {
                Val::array(n.children.iter().map(|&c| Val::Node(c)).collect())
            }
            "firstChild" | "firstElementChild" => {
                n.children.first().copied().map(Val::Node).unwrap_or(Val::Null)
            }
            "lastChild" | "lastElementChild" => {
                n.children.last().copied().map(Val::Node).unwrap_or(Val::Null)
            }
            "nextElementSibling" | "nextSibling" => self.sibling(idx, 1),
            "previousElementSibling" | "previousSibling" => self.sibling(idx, -1),
            "childElementCount" => Val::Num(n.children.len() as f64),
            "offsetWidth" | "offsetHeight" | "scrollTop" | "scrollHeight" | "clientWidth"
            | "clientHeight" => Val::Num(0.0),
            // methods
            _ => Val::Native(Native::Method(Box::new(Val::Node(idx)), Rc::from(prop))),
        }
    }

    fn sibling(&self, idx: usize, dir: i32) -> Val {
        if let Some(p) = self.dom.nodes[idx].parent {
            let kids = &self.dom.nodes[p].children;
            if let Some(pos) = kids.iter().position(|&c| c == idx) {
                let np = pos as i32 + dir;
                if np >= 0 && (np as usize) < kids.len() {
                    return Val::Node(kids[np as usize]);
                }
            }
        }
        Val::Null
    }

    fn set_node_member(&mut self, idx: usize, prop: &str, v: Val) {
        match prop {
            "innerHTML" | "outerHTML" => self.dom.set_inner_html(idx, &v.to_str()),
            "textContent" | "innerText" => self.dom.set_text_content(idx, &v.to_str()),
            "nodeValue" | "data" if self.dom.nodes[idx].is_text() => self.dom.nodes[idx].text = v.to_str(),
            "className" => self.dom.nodes[idx].set_attr("class", &v.to_str()),
            "id" => self.dom.nodes[idx].set_attr("id", &v.to_str()),
            "src" => self.dom.nodes[idx].set_attr("src", &v.to_str()),
            "href" => self.dom.nodes[idx].set_attr("href", &v.to_str()),
            "value" => self.dom.nodes[idx].set_attr("value", &v.to_str()),
            "content" => self.dom.nodes[idx].set_attr("content", &v.to_str()),
            "title" => self.dom.nodes[idx].set_attr("title", &v.to_str()),
            "hidden" => {
                if v.truthy() {
                    self.dom.nodes[idx].set_attr("hidden", "");
                } else {
                    self.dom.nodes[idx].attrs.retain(|(k, _)| k != "hidden");
                }
            }
            _ => {}
        }
    }

    // ---- host object members -----------------------------------------------

    fn host_member(&mut self, h: Host, prop: &str) -> Val {
        match h {
            Host::Document => match prop {
                "documentElement" => self
                    .dom
                    .get_by_tag("html")
                    .next()
                    .map(Val::Node)
                    .unwrap_or(Val::Node(self.dom.root)),
                "body" => self.dom.get_by_tag("body").next().map(Val::Node).unwrap_or(Val::Null),
                "head" => self.dom.get_by_tag("head").next().map(Val::Node).unwrap_or(Val::Null),
                "title" => Val::str(self.dom.get_by_tag("title").next().map(|i| self.dom.text_content(i)).unwrap_or_default()),
                "readyState" => Val::str("complete"),
                "cookie" => Val::str(""),
                _ => Val::Native(Native::Method(Box::new(Val::Host(Host::Document)), Rc::from(prop))),
            },
            Host::Window => match prop {
                "location" => Val::Host(Host::Location),
                "history" => Val::Host(Host::History),
                "document" => Val::Host(Host::Document),
                "localStorage" => Val::Host(Host::LocalStorage),
                "innerWidth" => Val::Num(1024.0),
                "innerHeight" => Val::Num(768.0),
                "devicePixelRatio" => Val::Num(1.0),
                "scrollY" | "scrollX" | "pageYOffset" => Val::Num(0.0),
                // `window.event` is the legacy global event; it is `undefined`
                // outside of event dispatch. React's getCurrentEventPriority
                // branches on `window.event === undefined`, so it MUST be a real
                // undefined (not the phantom method-native the `_` arm returns).
                "event" => Val::Undef,
                // window.indexedDB resolves to the polyfilled global.
                "indexedDB" => self.global_val("indexedDB").unwrap_or(Val::Undef),
                // Reading window.<x> for any global that was defined (e.g. a UMD
                // bundle's `window.React = {}`) returns that global. Method names
                // (addEventListener, scrollTo…) are dispatched via builtin_method
                // before this read, so returning the global here is safe.
                _ => match self.global.borrow().vars.get(prop) {
                    Some(v) => v.clone(),
                    None => Val::Native(Native::Method(Box::new(Val::Host(Host::Window)), Rc::from(prop))),
                },
            },
            Host::Math => match prop {
                "PI" => Val::Num(core::f64::consts::PI),
                "E" => Val::Num(core::f64::consts::E),
                _ => Val::Native(Native::Method(Box::new(Val::Host(Host::Math)), Rc::from(prop))),
            },
            Host::Location => match prop {
                "href" => Val::str("https://henryratterman.com/"),
                "pathname" => Val::str("/"),
                "hostname" => Val::str("henryratterman.com"),
                "host" => Val::str("henryratterman.com"),
                "protocol" => Val::str("https:"),
                "hash" => Val::str(""),
                "search" => Val::str(""),
                "origin" => Val::str("https://henryratterman.com"),
                _ => Val::Native(Native::Method(Box::new(Val::Host(Host::Location)), Rc::from(prop))),
            },
            Host::LocalStorage | Host::SessionStorage => {
                let local = matches!(h, Host::LocalStorage);
                let origin = crate::browser::current_origin();
                if prop == "length" {
                    return Val::Num(crate::browser::storage_keys(local, &origin).len() as f64);
                }
                if matches!(prop, "getItem" | "setItem" | "removeItem" | "clear" | "key") {
                    return Val::Native(Native::Method(Box::new(Val::Host(h)), Rc::from(prop)));
                }
                // Direct property access: localStorage.foo === localStorage.getItem('foo')
                crate::browser::storage_get(local, &origin, prop).map(Val::str).unwrap_or(Val::Undef)
            }
            Host::Canvas(n) => {
                // `ctx.canvas` reflects back the context; fillStyle/lineWidth/etc.
                // read the stored state; everything else is a drawing method.
                if prop == "canvas" {
                    return Val::Host(Host::Canvas(n));
                }
                if let Some(s) = self.canvases.get(n).and_then(|c| c.get_prop(prop)) {
                    // numeric props as numbers, colors/strings as strings
                    return match prop {
                        "lineWidth" | "globalAlpha" | "width" | "height" => Val::Num(s.parse().unwrap_or(0.0)),
                        _ => Val::str(s),
                    };
                }
                Val::Native(Native::Method(Box::new(Val::Host(Host::Canvas(n))), Rc::from(prop)))
            }
            Host::WebGl(g) => {
                // WebGL enum constants + a few readable properties; everything
                // else is a gl.* method.
                if let Some(v) = webgl_const(prop) {
                    return Val::Num(v);
                }
                match prop {
                    "canvas" => self.webgl.get(g).map(|c| Val::Host(Host::Canvas(c.canvas))).unwrap_or(Val::Null),
                    "drawingBufferWidth" => Val::Num(self.webgl.get(g).map(|c| c.w as f64).unwrap_or(0.0)),
                    "drawingBufferHeight" => Val::Num(self.webgl.get(g).map(|c| c.h as f64).unwrap_or(0.0)),
                    _ => Val::Native(Native::Method(Box::new(Val::Host(Host::WebGl(g))), Rc::from(prop))),
                }
            }
            Host::ClassList(_) | Host::Console | Host::History
            | Host::Style(_) | Host::Dataset(_) => {
                if let Host::Style(idx) = h {
                    return Val::str(self.dom.get_style(idx, prop));
                }
                if let Host::Dataset(idx) = h {
                    let key = alloc::format!("data-{}", kebab(prop));
                    return Val::str(self.dom.nodes[idx].attr(&key).unwrap_or("").to_string());
                }
                Val::Native(Native::Method(Box::new(Val::Host(h)), Rc::from(prop)))
            }
        }
    }

    // ---- builtin method dispatch -------------------------------------------
    // Returns Some(result) if handled, None to fall through to user props.

    fn builtin_method(&mut self, recv: &Val, name: &str, args: &[Val]) -> Result<Option<Val>, Val> {
        let a0 = || args.first().cloned().unwrap_or(Val::Undef);
        match recv {
            // Function.prototype.call / apply / bind.
            Val::Func(..) if matches!(name, "call" | "apply" | "bind") => {
                let this_arg = args.first().cloned().unwrap_or(Val::Undef);
                match name {
                    "call" => {
                        let rest: Vec<Val> = args.iter().skip(1).cloned().collect();
                        Ok(Some(self.call(recv.clone(), this_arg, rest)?))
                    }
                    "apply" => {
                        let rest = match args.get(1) {
                            Some(Val::Array(a)) => a.borrow().clone(),
                            _ => Vec::new(),
                        };
                        Ok(Some(self.call(recv.clone(), this_arg, rest)?))
                    }
                    // bind: return a native closure capturing (func, this, preargs).
                    _ => {
                        let pre: Vec<Val> = args.iter().skip(1).cloned().collect();
                        let mut o = Obj::new();
                        o.insert("__bound_fn".into(), recv.clone());
                        o.insert("__bound_this".into(), this_arg);
                        o.insert("__bound_args".into(), Val::array(pre));
                        Ok(Some(Val::object(o)))
                    }
                }
            }
            Val::Host(Host::Document) => Ok(Some(self.document_method(name, args)?)),
            Val::Host(Host::Window) => Ok(Some(self.window_method(name, args)?)),
            Val::Host(Host::Console) => {
                let parts: Vec<String> = args.iter().map(|v| v.to_str()).collect();
                self.errors.push(alloc::format!("[console] {}", parts.join(" ")));
                Ok(Some(Val::Undef))
            }
            Val::Host(Host::Math) => Ok(Some(math_method(name, args))),
            Val::Host(Host::LocalStorage) => Ok(Some(self.storage_method(true, name, args))),
            Val::Host(Host::SessionStorage) => Ok(Some(self.storage_method(false, name, args))),
            Val::Host(Host::History) => Ok(Some(Val::Undef)),
            Val::Host(Host::Location) => Ok(Some(Val::Undef)),
            Val::Host(Host::Canvas(n)) => Ok(Some(self.canvas_method(*n, name, args))),
            Val::Host(Host::WebGl(g)) => Ok(Some(self.webgl_method(*g, name, args))),
            Val::Host(Host::ClassList(idx)) => Ok(Some(self.classlist_method(*idx, name, args))),
            Val::Host(Host::Style(_)) => Ok(Some(Val::Undef)),
            Val::Node(idx) => Ok(self.node_method(*idx, name, args)?),
            Val::Array(a) => Ok(self.array_method(a.clone(), name, args)?),
            Val::Str(s) => Ok(Some(str_method(s, name, args))),
            Val::Num(n) => Ok(Some(num_method(*n, name, args))),
            Val::Object(o) => self.object_method(o.clone(), name, args),
            _ => Ok(None),
        }
    }

    /// localStorage / sessionStorage methods, persisted via the browser (which
    /// keeps them across the per-render interpreter instances).
    fn storage_method(&mut self, local: bool, name: &str, args: &[Val]) -> Val {
        let origin = crate::browser::current_origin();
        let a0 = args.first().map(|v| v.to_str()).unwrap_or_default();
        match name {
            "getItem" => crate::browser::storage_get(local, &origin, &a0).map(Val::str).unwrap_or(Val::Null),
            "setItem" => {
                let v = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                crate::browser::storage_set(local, &origin, &a0, &v);
                Val::Undef
            }
            "removeItem" => {
                crate::browser::storage_remove(local, &origin, &a0);
                Val::Undef
            }
            "clear" => {
                crate::browser::storage_clear(local, &origin);
                Val::Undef
            }
            "key" => {
                let keys = crate::browser::storage_keys(local, &origin);
                keys.get(a0.parse::<usize>().unwrap_or(usize::MAX)).cloned().map(Val::str).unwrap_or(Val::Null)
            }
            _ => Val::Undef,
        }
    }

    fn document_method(&mut self, name: &str, args: &[Val]) -> Result<Val, Val> {
        let a0 = args.first().map(|v| v.to_str()).unwrap_or_default();
        match name {
            "getElementById" => Ok(self.dom.get_by_id(&a0).map(Val::Node).unwrap_or(Val::Null)),
            "querySelector" => Ok(self.dom.query(&a0).map(Val::Node).unwrap_or(Val::Null)),
            "querySelectorAll" => {
                Ok(Val::array(self.dom.query_all(&a0).into_iter().map(Val::Node).collect()))
            }
            "getElementsByClassName" => Ok(Val::array(
                self.dom.query_all(&alloc::format!(".{a0}")).into_iter().map(Val::Node).collect(),
            )),
            "getElementsByTagName" => {
                Ok(Val::array(self.dom.get_by_tag(&a0).map(Val::Node).collect()))
            }
            "createElement" => {
                CE_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                Ok(Val::Node(self.dom.create_element(&a0)))
            }
            // createElementNS(ns, tag) — ignore the namespace, use the tag.
            "createElementNS" => {
                let tag = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                Ok(Val::Node(self.dom.create_element(&tag)))
            }
            "createTextNode" => Ok(Val::Node(self.dom.create_text(&a0))),
            "createComment" => {
                let idx = self.dom.create_text(&a0);
                self.dom.nodes[idx].tag = String::from("#comment");
                Ok(Val::Node(idx))
            }
            "createDocumentFragment" => Ok(Val::Node(self.dom.create_fragment())),
            "addEventListener" => {
                if let Some(h) = args.get(1) {
                    self.listeners.push(Listener { node: self.dom.root, event: a0, handler: h.clone() });
                }
                Ok(Val::Undef)
            }
            "dispatchEvent" => {
                self.dispatch_to_node(self.dom.root, args.first().unwrap_or(&Val::Undef))?;
                Ok(Val::Bool(true))
            }
            "removeEventListener" | "write" | "createEvent" => Ok(Val::Undef),
            _ => Ok(Val::Undef),
        }
    }

    fn window_method(&mut self, name: &str, args: &[Val]) -> Result<Val, Val> {
        match name {
            "matchMedia" => {
                let mut o = Obj::new();
                let q = args.first().map(|v| v.to_str()).unwrap_or_default();
                // honour prefers-color-scheme: dark? default light.
                o.insert("matches".into(), Val::Bool(q.contains("dark") && false));
                o.insert("media".into(), Val::str(q));
                o.insert("addListener".into(), Val::Native(Native::Global(Rc::from("noop"))));
                o.insert("addEventListener".into(), Val::Native(Native::Global(Rc::from("noop"))));
                Ok(Val::object(o))
            }
            "requestAnimationFrame" | "setTimeout" | "setInterval" => {
                if let Some(f) = args.first() {
                    self.deferred.push((f.clone(), Vec::new()));
                }
                Ok(Val::Num(0.0))
            }
            "addEventListener" => {
                let ev = args.first().map(|v| v.to_str()).unwrap_or_default();
                if let Some(h) = args.get(1) {
                    self.listeners.push(Listener { node: self.dom.root, event: ev, handler: h.clone() });
                }
                Ok(Val::Undef)
            }
            "getComputedStyle" => {
                let mut o = Obj::new();
                o.insert("getPropertyValue".into(), Val::Native(Native::Global(Rc::from("noop"))));
                Ok(Val::object(o))
            }
            "dispatchEvent" => {
                self.dispatch_to_node(self.dom.root, args.first().unwrap_or(&Val::Undef))?;
                Ok(Val::Bool(true))
            }
            // postMessage(data): deliver to window 'message' listeners on next tick.
            "postMessage" => {
                let data = args.first().cloned().unwrap_or(Val::Undef);
                let mut ev = Obj::new();
                ev.insert("type".into(), Val::str("message"));
                ev.insert("data".into(), data);
                let evv = Val::object(ev);
                let handlers: Vec<Val> = self
                    .listeners
                    .iter()
                    .filter(|l| l.node == self.dom.root && l.event == "message")
                    .map(|l| l.handler.clone())
                    .collect();
                for h in handlers {
                    self.deferred.push((h, alloc::vec![evv.clone()]));
                }
                Ok(Val::Undef)
            }
            // window.open(url, target, features): signal the browser/WM to open
            // a second browser window, and return a window proxy with
            // postMessage/opener/closed/location so cross-window scripts work.
            "open" => {
                let url = args.first().map(|v| v.to_str()).unwrap_or_default();
                crate::browser::request_new_window(&url);
                let mut loc = Obj::new();
                loc.insert("href".into(), Val::str(url.clone()));
                loc.insert("pathname".into(), Val::str(
                    url.split("://").nth(1).and_then(|r| r.split_once('/')).map(|(_, p)| alloc::format!("/{p}")).unwrap_or_else(|| String::from("/"))
                ));
                let mut w = Obj::new();
                w.insert("__window".into(), Val::Bool(true));
                w.insert("closed".into(), Val::Bool(false));
                w.insert("opener".into(), Val::Host(Host::Window));
                w.insert("location".into(), Val::object(loc));
                w.insert("name".into(), Val::str(args.get(1).map(|v| v.to_str()).unwrap_or_default()));
                // postMessage into the opened window delivers a 'message' event
                // back to this window's listeners (the child echoes to its opener).
                w.insert("postMessage".into(), Val::Native(Native::Global(Rc::from("__win_post"))));
                w.insert("close".into(), Val::Native(Native::Global(Rc::from("noop"))));
                w.insert("focus".into(), Val::Native(Native::Global(Rc::from("noop"))));
                w.insert("blur".into(), Val::Native(Native::Global(Rc::from("noop"))));
                Ok(Val::object(w))
            }
            "scrollTo" | "scroll" | "scrollBy" | "removeEventListener" | "alert" | "focus" => {
                Ok(Val::Undef)
            }
            _ => Ok(Val::Undef),
        }
    }

    fn classlist_method(&mut self, idx: usize, name: &str, args: &[Val]) -> Val {
        let a0 = args.first().map(|v| v.to_str()).unwrap_or_default();
        match name {
            "add" => {
                for a in args {
                    self.dom.class_add(idx, &a.to_str());
                }
                Val::Undef
            }
            "remove" => {
                for a in args {
                    self.dom.class_remove(idx, &a.to_str());
                }
                Val::Undef
            }
            "toggle" => Val::Bool(self.dom.class_toggle(idx, &a0)),
            "contains" => Val::Bool(self.dom.class_contains(idx, &a0)),
            _ => Val::Undef,
        }
    }

    /// Fire registered listeners for an event, bubbling from `start` up through
    /// its ancestors to the document root. `event` is the Event object (its
    /// `type` selects matching listeners).
    fn dispatch_to_node(&mut self, start: usize, event: &Val) -> Result<(), Val> {
        let ty = match event {
            Val::Object(o) => o.borrow().get("type").map(|v| v.to_str()).unwrap_or_default(),
            other => other.to_str(),
        };
        // Build the bubble path: target, parents…, root.
        let mut path = alloc::vec![start];
        let mut cur = start;
        while let Some(p) = self.dom.nodes[cur].parent {
            path.push(p);
            cur = p;
        }
        if !path.contains(&self.dom.root) {
            path.push(self.dom.root);
        }
        for node in path {
            let handlers: Vec<Val> = self
                .listeners
                .iter()
                .filter(|l| l.node == node && l.event == ty)
                .map(|l| l.handler.clone())
                .collect();
            for h in handlers {
                self.call(h, Val::Node(node), alloc::vec![event.clone()])?;
            }
        }
        Ok(())
    }

    fn node_method(&mut self, idx: usize, name: &str, args: &[Val]) -> Result<Option<Val>, Val> {
        let a0 = args.first().cloned().unwrap_or(Val::Undef);
        let s0 = a0.to_str();
        let r = match name {
            "appendChild" => {
                if let Val::Node(c) = a0 {
                    self.dom.append_child(idx, c);
                }
                a0
            }
            "append" => {
                for a in args {
                    if let Val::Node(c) = a {
                        self.dom.append_child(idx, *c);
                    } else {
                        let t = self.dom.create_text(&a.to_str());
                        self.dom.append_child(idx, t);
                    }
                }
                Val::Undef
            }
            "insertBefore" => {
                if let Val::Node(c) = a0 {
                    let reference = match args.get(1) {
                        Some(Val::Node(r)) => Some(*r),
                        _ => None,
                    };
                    self.dom.insert_before(idx, c, reference);
                }
                a0
            }
            "removeChild" => {
                if let Val::Node(c) = a0 {
                    self.dom.remove_child(idx, c);
                }
                a0
            }
            "replaceChild" => {
                // replaceChild(newChild, oldChild)
                if let (Val::Node(nw), Some(Val::Node(old))) = (a0.clone(), args.get(1)) {
                    self.dom.insert_before(idx, nw, Some(*old));
                    self.dom.remove_child(idx, *old);
                }
                args.get(1).cloned().unwrap_or(Val::Undef)
            }
            "dispatchEvent" => {
                self.dispatch_to_node(idx, &a0)?;
                Val::Bool(true)
            }
            "remove" => {
                if let Some(p) = self.dom.nodes[idx].parent {
                    self.dom.nodes[p].children.retain(|&x| x != idx);
                }
                Val::Undef
            }
            "setAttribute" => {
                self.dom.nodes[idx].set_attr(&s0, &args.get(1).map(|v| v.to_str()).unwrap_or_default());
                Val::Undef
            }
            "getAttribute" => self
                .dom
                .nodes[idx]
                .attr(&s0)
                .map(|v| Val::str(v.to_string()))
                .unwrap_or(Val::Null),
            "hasAttribute" => Val::Bool(self.dom.nodes[idx].attr(&s0).is_some()),
            "removeAttribute" => {
                self.dom.nodes[idx].attrs.retain(|(k, _)| k != &s0);
                Val::Undef
            }
            "querySelector" => self.query_within(idx, &s0).into_iter().next().map(Val::Node).unwrap_or(Val::Null),
            "querySelectorAll" => Val::array(self.query_within(idx, &s0).into_iter().map(Val::Node).collect()),
            "closest" => self.closest(idx, &s0),
            "contains" => Val::Bool(matches!(a0, Val::Node(c) if self.is_descendant(idx, c))),
            "addEventListener" => {
                if let Some(h) = args.get(1) {
                    self.listeners.push(Listener { node: idx, event: s0, handler: h.clone() });
                }
                Val::Undef
            }
            "removeEventListener" | "focus" | "blur" | "click" | "scrollIntoView" | "preventDefault"
            | "stopPropagation" | "setProperty" => Val::Undef,
            "getBoundingClientRect" => {
                let mut o = Obj::new();
                for k in ["top", "left", "right", "bottom", "width", "height", "x", "y"] {
                    o.insert(k.into(), Val::Num(0.0));
                }
                Val::object(o)
            }
            "matches" => Val::Bool({
                let n = &self.dom.nodes[idx];
                self.dom.query_all(&s0).contains(&idx) || self.dom.has_class(n, s0.trim_start_matches('.'))
            }),
            "cloneNode" => Val::Node(idx),
            "getContext" => {
                let w = self.dom.nodes[idx].attr("width").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(300);
                let h = self.dom.nodes[idx].attr("height").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(150);
                // WebGL: allocate a framebuffer canvas + a GL context over it.
                if s0.starts_with("webgl") || s0.starts_with("experimental-webgl") {
                    let cv = match self.dom.nodes[idx].attr("__cvs").and_then(|v| v.parse::<usize>().ok()) {
                        Some(n) if n < self.canvases.len() => n,
                        _ => {
                            let n = self.canvases.len();
                            self.canvases.push(super::canvas::Canvas::new(w, h));
                            self.dom.nodes[idx].set_attr("__cvs", &alloc::format!("{n}"));
                            n
                        }
                    };
                    let g = self.webgl.len();
                    self.webgl.push(super::webgl::GlContext::new(cv, w, h));
                    self.dom.nodes[idx].set_attr("__gl", &alloc::format!("{g}"));
                    return Ok(Some(Val::Host(Host::WebGl(g))));
                }
                if !s0.starts_with("2d") {
                    return Ok(Some(Val::Null));
                }
                let n = match self.dom.nodes[idx].attr("__cvs").and_then(|v| v.parse::<usize>().ok()) {
                    Some(n) if n < self.canvases.len() => n,
                    _ => {
                        let n = self.canvases.len();
                        self.canvases.push(super::canvas::Canvas::new(w, h));
                        self.dom.nodes[idx].set_attr("__cvs", &alloc::format!("{n}"));
                        n
                    }
                };
                Val::Host(Host::Canvas(n))
            }
            _ => return Ok(None),
        };
        Ok(Some(r))
    }

    /// HTML5 canvas 2D context methods, dispatched on the canvas index `n`.
    fn canvas_method(&mut self, n: usize, name: &str, args: &[Val]) -> Val {
        let f = |i: usize| args.get(i).map(|v| v.as_num()).unwrap_or(0.0);
        // drawImage / putImageData read other objects before borrowing the canvas.
        match name {
            "measureText" => {
                let s = args.first().map(|v| v.to_str()).unwrap_or_default();
                let w = self.canvases.get(n).map(|c| c.measure_text(&s)).unwrap_or(0.0);
                let mut o = Obj::new();
                o.insert("width".into(), Val::Num(w));
                return Val::object(o);
            }
            "getImageData" => {
                let (data, w, h) = match self.canvases.get(n) {
                    Some(c) => c.get_image_data(f(0), f(1), f(2), f(3)),
                    None => return Val::Undef,
                };
                let mut o = Obj::new();
                o.insert("width".into(), Val::Num(w as f64));
                o.insert("height".into(), Val::Num(h as f64));
                o.insert("data".into(), Val::array(data.into_iter().map(|b| Val::Num(b as f64)).collect()));
                return Val::object(o);
            }
            "putImageData" => {
                if let Some(Val::Object(o)) = args.first() {
                    let ob = o.borrow();
                    let w = ob.get("width").map(|v| v.as_num() as usize).unwrap_or(0);
                    let h = ob.get("height").map(|v| v.as_num() as usize).unwrap_or(0);
                    let bytes: Vec<u8> = match ob.get("data") {
                        Some(Val::Array(a)) => a.borrow().iter().map(|v| v.as_num() as u8).collect(),
                        _ => Vec::new(),
                    };
                    drop(ob);
                    if let Some(c) = self.canvases.get_mut(n) {
                        c.put_image_data(&bytes, w, h, f(1), f(2));
                    }
                }
                return Val::Undef;
            }
            "drawImage" => {
                // Only canvas->canvas is supported (an <img>'s pixels live in the
                // browser, not the interpreter).
                let src = match args.first() {
                    Some(Val::Host(Host::Canvas(m))) if *m < self.canvases.len() => {
                        Some(self.canvases[*m].snapshot())
                    }
                    _ => None,
                };
                if let (Some((buf, sw, sh)), Some(c)) = (src, self.canvases.get_mut(n)) {
                    let (dx, dy, dw, dh) = match args.len() {
                        n if n >= 9 => (f(5), f(6), f(7), f(8)),
                        n if n >= 5 => (f(1), f(2), f(3), f(4)),
                        _ => (f(1), f(2), sw as f64, sh as f64),
                    };
                    c.draw_image_buf(&buf, sw, sh, dx, dy, dw, dh);
                }
                return Val::Undef;
            }
            _ => {}
        }
        let Some(c) = self.canvases.get_mut(n) else { return Val::Undef };
        match name {
            "fillRect" => c.fill_rect(f(0), f(1), f(2), f(3)),
            "strokeRect" => c.stroke_rect(f(0), f(1), f(2), f(3)),
            "clearRect" => c.clear_rect(f(0), f(1), f(2), f(3)),
            "beginPath" => c.begin_path(),
            "closePath" => c.close_path(),
            "moveTo" => c.move_to(f(0), f(1)),
            "lineTo" => c.line_to(f(0), f(1)),
            "rect" => c.rect(f(0), f(1), f(2), f(3)),
            "arc" => c.arc(f(0), f(1), f(2), f(3), f(4), args.get(5).map(|v| v.truthy()).unwrap_or(false)),
            "arcTo" => c.arc_to(f(0), f(1), f(2), f(3), f(4)),
            "ellipse" => c.arc(f(0), f(1), f(2).max(f(3)), f(4), f(5), args.get(7).map(|v| v.truthy()).unwrap_or(false)),
            "bezierCurveTo" => c.bezier_curve_to(f(0), f(1), f(2), f(3), f(4), f(5)),
            "quadraticCurveTo" => c.quadratic_curve_to(f(0), f(1), f(2), f(3)),
            "fill" => c.fill(),
            "stroke" => c.stroke(),
            "fillText" => c.fill_text(&args.first().map(|v| v.to_str()).unwrap_or_default(), f(1), f(2)),
            "strokeText" => c.stroke_text(&args.first().map(|v| v.to_str()).unwrap_or_default(), f(1), f(2)),
            "save" => c.save(),
            "restore" => c.restore(),
            "translate" => c.translate(f(0), f(1)),
            "scale" => c.scale(f(0), f(1)),
            "rotate" => c.rotate(f(0)),
            "transform" => c.transform(f(0), f(1), f(2), f(3), f(4), f(5)),
            "setTransform" => c.set_transform(f(0), f(1), f(2), f(3), f(4), f(5)),
            "resetTransform" => c.reset_transform(),
            // No-op context methods sites call but we don't need.
            "setLineDash" | "getLineDash" | "clip" | "createLinearGradient"
            | "createRadialGradient" | "createPattern" | "closePathAll" => {}
            _ => {}
        }
        Val::Undef
    }

    /// WebGL 1.0 method dispatch on GL context index `g`. Shaders/programs are
    /// plain JS objects; buffers/locations are integer handles into the context.
    fn webgl_method(&mut self, g: usize, name: &str, args: &[Val]) -> Val {
        let num = |i: usize| args.get(i).map(|v| v.as_num()).unwrap_or(0.0);
        match name {
            "createShader" => {
                let mut o = Obj::new();
                o.insert("__sh".into(), Val::Bool(true));
                o.insert("type".into(), Val::Num(num(0)));
                o.insert("src".into(), Val::str(""));
                Val::object(o)
            }
            "shaderSource" => {
                if let (Some(Val::Object(sh)), Some(src)) = (args.first(), args.get(1)) {
                    sh.borrow_mut().insert("src".into(), Val::str(src.to_str()));
                }
                Val::Undef
            }
            "compileShader" | "linkProgram" | "validateProgram" => Val::Undef,
            "getShaderParameter" | "getProgramParameter" => Val::Bool(true),
            "getShaderInfoLog" | "getProgramInfoLog" => Val::str(""),
            "createProgram" => {
                let mut o = Obj::new();
                o.insert("__prog".into(), Val::Bool(true));
                o.insert("shaders".into(), Val::array(Vec::new()));
                Val::object(o)
            }
            "attachShader" => {
                if let (Some(Val::Object(p)), Some(sh)) = (args.first(), args.get(1)) {
                    if let Some(Val::Array(list)) = p.borrow().get("shaders") {
                        list.borrow_mut().push(sh.clone());
                    }
                }
                Val::Undef
            }
            "useProgram" => {
                // copy each attached shader's source into the GL context by type
                let mut vsrc = None;
                let mut fsrc = None;
                if let Some(Val::Object(p)) = args.first() {
                    if let Some(Val::Array(list)) = p.borrow().get("shaders") {
                        for sh in list.borrow().iter() {
                            if let Val::Object(o) = sh {
                                let b = o.borrow();
                                let ty = b.get("type").map(|v| v.as_num()).unwrap_or(0.0);
                                let src = b.get("src").map(|v| v.to_str()).unwrap_or_default();
                                if ty == 35633.0 { vsrc = Some(src); } else { fsrc = Some(src); }
                            }
                        }
                    }
                }
                if let Some(gl) = self.webgl.get_mut(g) {
                    if let Some(s) = vsrc { gl.vert_src = s; }
                    if let Some(s) = fsrc { gl.frag_src = s; }
                }
                Val::Undef
            }
            "createBuffer" => {
                if let Some(gl) = self.webgl.get_mut(g) {
                    gl.buffers.push(super::webgl::Buffer { data: Vec::new() });
                    return Val::Num((gl.buffers.len() - 1) as f64);
                }
                Val::Num(0.0)
            }
            "bindBuffer" => {
                if let Some(gl) = self.webgl.get_mut(g) {
                    gl.bound_array = num(1) as usize;
                }
                Val::Undef
            }
            "bufferData" => {
                // bufferData(target, data, usage) — data is an array of numbers
                // (plain Array or our typed-array-as-Array).
                let data: Vec<f32> = match args.get(1) {
                    Some(Val::Array(a)) => a.borrow().iter().map(|v| v.as_num() as f32).collect(),
                    _ => Vec::new(),
                };
                if let Some(gl) = self.webgl.get_mut(g) {
                    let b = gl.bound_array;
                    if let Some(buf) = gl.buffers.get_mut(b) { buf.data = data; }
                }
                Val::Undef
            }
            "getAttribLocation" => {
                let nm = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                Val::Num(self.webgl.get_mut(g).map(|gl| gl.attrib_location(&nm) as f64).unwrap_or(-1.0))
            }
            "enableVertexAttribArray" => {
                let loc = num(0) as usize;
                if let Some(gl) = self.webgl.get_mut(g) {
                    gl.attribs.entry(loc).or_insert(super::webgl::AttribPtr { buffer: 0, size: 0, stride: 0, offset: 0, enabled: false }).enabled = true;
                }
                Val::Undef
            }
            "disableVertexAttribArray" => {
                let loc = num(0) as usize;
                if let Some(gl) = self.webgl.get_mut(g) {
                    if let Some(ap) = gl.attribs.get_mut(&loc) { ap.enabled = false; }
                }
                Val::Undef
            }
            "vertexAttribPointer" => {
                // (loc, size, type, normalized, stride_bytes, offset_bytes)
                let loc = num(0) as usize;
                let size = num(1) as usize;
                let stride = (num(4) as usize) / 4; // bytes -> floats
                let offset = (num(5) as usize) / 4;
                if let Some(gl) = self.webgl.get_mut(g) {
                    let buffer = gl.bound_array;
                    let enabled = gl.attribs.get(&loc).map(|a| a.enabled).unwrap_or(true);
                    gl.attribs.insert(loc, super::webgl::AttribPtr { buffer, size, stride, offset, enabled });
                }
                Val::Undef
            }
            "getUniformLocation" => {
                let nm = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                Val::Num(self.webgl.get_mut(g).map(|gl| gl.uniform_location(&nm) as f64).unwrap_or(-1.0))
            }
            "uniformMatrix4fv" => {
                let loc = num(0) as usize;
                let m: Vec<f32> = match args.get(2) {
                    Some(Val::Array(a)) => a.borrow().iter().map(|v| v.as_num() as f32).collect(),
                    _ => Vec::new(),
                };
                if m.len() >= 16 {
                    let mut arr = [0.0f32; 16];
                    arr.copy_from_slice(&m[..16]);
                    if let Some(gl) = self.webgl.get_mut(g) {
                        if let Some(nm) = gl.uniform_locs.get(loc).cloned() {
                            gl.uniforms.insert(nm, super::webgl::Glsl::M4(arr));
                        }
                    }
                }
                Val::Undef
            }
            "uniform4f" | "uniform3f" | "uniform2f" | "uniform1f" => {
                let n = name.as_bytes()[7] - b'0';
                let comps: Vec<f32> = (0..n).map(|i| num(1 + i as usize) as f32).collect();
                if let Some(gl) = self.webgl.get_mut(g) {
                    let loc = num(0) as usize;
                    if let Some(nm) = gl.uniform_locs.get(loc).cloned() {
                        gl.uniforms.insert(nm, super::webgl::Glsl::vec(&comps));
                    }
                }
                Val::Undef
            }
            "uniform1i" => {
                if let Some(gl) = self.webgl.get_mut(g) {
                    let loc = num(0) as usize;
                    if let Some(nm) = gl.uniform_locs.get(loc).cloned() {
                        gl.uniforms.insert(nm, super::webgl::Glsl::F(num(1) as f32));
                    }
                }
                Val::Undef
            }
            "clearColor" => {
                if let Some(gl) = self.webgl.get_mut(g) {
                    gl.clear = [num(0) as f32, num(1) as f32, num(2) as f32, num(3) as f32];
                }
                Val::Undef
            }
            "clear" => {
                let cv = self.webgl.get(g).map(|gl| gl.canvas);
                if let (Some(cv), Some(gl)) = (cv, self.webgl.get(g)) {
                    let clear = gl.clear;
                    let _ = clear;
                    let g_ctx = &self.webgl[g];
                    let px = self.canvases[cv].px_mut();
                    g_ctx.clear(px);
                }
                Val::Undef
            }
            "drawArrays" => {
                let count = num(2) as usize;
                let cv = self.webgl.get(g).map(|gl| gl.canvas);
                if let Some(cv) = cv {
                    if cv < self.canvases.len() {
                        let g_ctx = &self.webgl[g];
                        let px = self.canvases[cv].px_mut();
                        g_ctx.draw_arrays(px, count);
                    }
                }
                Val::Undef
            }
            "drawElements" => Val::Undef,
            "createTexture" | "createFramebuffer" | "createRenderbuffer" => {
                let mut o = Obj::new();
                o.insert("__tex".into(), Val::Bool(true));
                Val::object(o)
            }
            "getParameter" => Val::Num(0.0),
            "getExtension" => Val::Null,
            // viewport / enable / blend / depth / texture state — accepted, no-op
            _ => Val::Undef,
        }
    }

    fn array_method(&mut self, a: Rc<RefCell<Vec<Val>>>, name: &str, args: &[Val]) -> Result<Option<Val>, Val> {
        let a0 = args.first().cloned().unwrap_or(Val::Undef);
        let r = match name {
            "push" => {
                for x in args {
                    a.borrow_mut().push(x.clone());
                }
                Val::Num(a.borrow().len() as f64)
            }
            "pop" => a.borrow_mut().pop().unwrap_or(Val::Undef),
            "shift" => {
                let mut b = a.borrow_mut();
                if b.is_empty() {
                    Val::Undef
                } else {
                    b.remove(0)
                }
            }
            "unshift" => {
                for (i, x) in args.iter().enumerate() {
                    a.borrow_mut().insert(i, x.clone());
                }
                Val::Num(a.borrow().len() as f64)
            }
            "join" => {
                let sep = if args.is_empty() { String::from(",") } else { a0.to_str() };
                let items: Vec<String> = a.borrow().iter().map(|v| {
                    if matches!(v, Val::Undef | Val::Null) { String::new() } else { v.to_str() }
                }).collect();
                Val::str(items.join(&sep))
            }
            "indexOf" => {
                let pos = a.borrow().iter().position(|v| loose_eq(v, &a0));
                Val::Num(pos.map(|p| p as f64).unwrap_or(-1.0))
            }
            "includes" => Val::Bool(a.borrow().iter().any(|v| loose_eq(v, &a0))),
            "slice" => {
                let b = a.borrow();
                let len = b.len() as i64;
                let start = norm_idx(args.first(), 0, len);
                let end = norm_idx(args.get(1), len, len);
                Val::array(b[start.min(len) as usize..end.clamp(start, len) as usize].to_vec())
            }
            "concat" => {
                let mut out = a.borrow().clone();
                for x in args {
                    out.extend(self.to_vec(x));
                }
                Val::array(out)
            }
            "reverse" => {
                a.borrow_mut().reverse();
                Val::Array(a.clone())
            }
            "fill" => {
                for slot in a.borrow_mut().iter_mut() {
                    *slot = a0.clone();
                }
                Val::Array(a.clone())
            }
            "flat" => {
                let mut out = Vec::new();
                for x in a.borrow().iter() {
                    if let Val::Array(inner) = x {
                        out.extend(inner.borrow().clone());
                    } else {
                        out.push(x.clone());
                    }
                }
                Val::array(out)
            }
            "forEach" => {
                let items = a.borrow().clone();
                for (i, x) in items.into_iter().enumerate() {
                    self.call(a0.clone(), Val::Undef, vec![x, Val::Num(i as f64), Val::Array(a.clone())])?;
                }
                Val::Undef
            }
            "map" => {
                let items = a.borrow().clone();
                let mut out = Vec::with_capacity(items.len());
                for (i, x) in items.into_iter().enumerate() {
                    out.push(self.call(a0.clone(), Val::Undef, vec![x, Val::Num(i as f64)])?);
                }
                Val::array(out)
            }
            "filter" => {
                let items = a.borrow().clone();
                let mut out = Vec::new();
                for (i, x) in items.into_iter().enumerate() {
                    if self.call(a0.clone(), Val::Undef, vec![x.clone(), Val::Num(i as f64)])?.truthy() {
                        out.push(x);
                    }
                }
                Val::array(out)
            }
            "find" => {
                let items = a.borrow().clone();
                let mut res = Val::Undef;
                for (i, x) in items.into_iter().enumerate() {
                    if self.call(a0.clone(), Val::Undef, vec![x.clone(), Val::Num(i as f64)])?.truthy() {
                        res = x;
                        break;
                    }
                }
                res
            }
            "findIndex" => {
                let items = a.borrow().clone();
                let mut res = -1.0;
                for (i, x) in items.into_iter().enumerate() {
                    if self.call(a0.clone(), Val::Undef, vec![x, Val::Num(i as f64)])?.truthy() {
                        res = i as f64;
                        break;
                    }
                }
                Val::Num(res)
            }
            "some" => {
                let items = a.borrow().clone();
                let mut res = false;
                for (i, x) in items.into_iter().enumerate() {
                    if self.call(a0.clone(), Val::Undef, vec![x, Val::Num(i as f64)])?.truthy() {
                        res = true;
                        break;
                    }
                }
                Val::Bool(res)
            }
            "every" => {
                let items = a.borrow().clone();
                let mut res = true;
                for (i, x) in items.into_iter().enumerate() {
                    if !self.call(a0.clone(), Val::Undef, vec![x, Val::Num(i as f64)])?.truthy() {
                        res = false;
                        break;
                    }
                }
                Val::Bool(res)
            }
            "reduce" => {
                let items = a.borrow().clone();
                let mut acc = args.get(1).cloned();
                let mut start = 0;
                if acc.is_none() {
                    acc = items.first().cloned();
                    start = 1;
                }
                let mut acc = acc.unwrap_or(Val::Undef);
                for (i, x) in items.into_iter().enumerate().skip(start) {
                    acc = self.call(a0.clone(), Val::Undef, vec![acc, x, Val::Num(i as f64)])?;
                }
                acc
            }
            "sort" => Val::Array(a.clone()),
            "splice" => Val::array(Vec::new()),
            _ => return Ok(None),
        };
        Ok(Some(r))
    }

    fn query_within(&self, root: usize, sel: &str) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_descendants(root, &mut out);
        out.into_iter().filter(|&i| {
            !self.dom.nodes[i].is_text() && self.dom.query_all(sel).contains(&i)
        }).collect()
    }

    fn collect_descendants(&self, idx: usize, out: &mut Vec<usize>) {
        for &c in &self.dom.nodes[idx].children {
            out.push(c);
            self.collect_descendants(c, out);
        }
    }

    fn closest(&self, mut idx: usize, sel: &str) -> Val {
        let matching = self.dom.query_all(sel);
        loop {
            if matching.contains(&idx) {
                return Val::Node(idx);
            }
            match self.dom.nodes[idx].parent {
                Some(p) => idx = p,
                None => return Val::Null,
            }
        }
    }

    fn is_descendant(&self, anc: usize, mut node: usize) -> bool {
        loop {
            if node == anc {
                return true;
            }
            match self.dom.nodes[node].parent {
                Some(p) => node = p,
                None => return false,
            }
        }
    }

    // ---- global functions --------------------------------------------------

    fn call_global(&mut self, name: &str, args: Vec<Val>) -> Result<Val, Val> {
        let a0 = args.first().cloned().unwrap_or(Val::Undef);
        Ok(match name {
            "noop" => Val::Undef,
            "setTimeout" | "setInterval" | "requestAnimationFrame" => {
                if let Some(f) = args.first() {
                    self.deferred.push((f.clone(), Vec::new()));
                }
                Val::Num(0.0)
            }
            "clearTimeout" | "clearInterval" | "cancelAnimationFrame" | "alert" => Val::Undef,
            "addEventListener" => {
                let ev = a0.to_str();
                if let Some(h) = args.get(1) {
                    self.listeners.push(Listener { node: self.dom.root, event: ev, handler: h.clone() });
                }
                Val::Undef
            }
            "parseInt" => {
                let s = a0.to_str();
                let s = s.trim();
                let radix = args.get(1).map(|v| v.as_num() as u32).filter(|&r| r != 0).unwrap_or(10);
                let digits: String = s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '+').collect();
                Val::Num(i64::from_str_radix(digits.trim_start_matches('+'), radix).map(|n| n as f64).unwrap_or(f64::NAN))
            }
            "parseFloat" => {
                let s = a0.to_str();
                let mut end = 0;
                let bytes = s.trim().as_bytes();
                while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.' || bytes[end] == b'-' || bytes[end] == b'+' || bytes[end] == b'e') {
                    end += 1;
                }
                Val::Num(s.trim()[..end].parse::<f64>().unwrap_or(f64::NAN))
            }
            "isNaN" => Val::Bool(a0.as_num().is_nan()),
            "isFinite" => Val::Bool(a0.as_num().is_finite()),
            "String" => Val::str(a0.to_str()),
            "Number" => Val::Num(a0.as_num()),
            "Boolean" => Val::Bool(a0.truthy()),
            "encodeURIComponent" | "decodeURIComponent" | "encodeURI" | "decodeURI" => Val::str(a0.to_str()),
            "Array" => {
                // Array(n) — single number is length; else wrap the args.
                if args.len() == 1 {
                    if let Val::Num(n) = a0 {
                        return Ok(Val::array(vec![Val::Undef; n.max(0.0) as usize]));
                    }
                }
                Val::array(args)
            }
            "Object" => if matches!(a0, Val::Object(_)) { a0 } else { Val::object(Obj::new()) },
            "Symbol" => Val::str(alloc::format!("Symbol({})", a0.to_str())),
            "structuredClone" => deep_clone(&a0),

            // ---- V8-parity builtins (M42 step 18) ----
            "TextEncoder.encode" => {
                let s = a0.to_str();
                Val::array(s.bytes().map(|b| Val::Num(b as f64)).collect())
            }
            "TextDecoder.decode" => {
                let bytes: Vec<u8> = match &a0 {
                    Val::Array(arr) => arr.borrow().iter().map(|v| v.as_num() as u8).collect(),
                    _ => Vec::new(),
                };
                Val::str(String::from_utf8_lossy(&bytes).into_owned())
            }
            "crypto.getRandomValues" => {
                // Fill the passed typed array with pseudo-random bytes (xorshift
                // seeded by a counter — deterministic but non-trivial, fine for
                // the no-real-entropy constraint). Returns the same array.
                if let Val::Array(arr) = &a0 {
                    let mut b = arr.borrow_mut();
                    let mut x = RNG_STATE.fetch_add(0x9E37_79B9, core::sync::atomic::Ordering::Relaxed) | 1;
                    for slot in b.iter_mut() {
                        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                        *slot = Val::Num((x & 0xff) as f64);
                    }
                }
                a0
            }
            "crypto.randomUUID" => {
                let mut x = RNG_STATE.fetch_add(0x1234_5678, core::sync::atomic::Ordering::Relaxed) | 1;
                let mut hex = String::new();
                for i in 0..32 {
                    x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                    hex.push(core::char::from_digit((x & 0xf) as u32, 16).unwrap());
                    if i == 7 || i == 11 || i == 15 || i == 19 { hex.push('-'); }
                }
                Val::str(hex)
            }
            "WeakRef.deref" => a0, // model: the ref never collected
            // Reflect.* mirror Object operations.
            "Reflect.get" => {
                let key = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                match &a0 { Val::Object(o) => o.borrow().get(&key).cloned().unwrap_or(Val::Undef), _ => Val::Undef }
            }
            "Reflect.set" => {
                if let Val::Object(o) = &a0 {
                    let key = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                    o.borrow_mut().insert(key, args.get(2).cloned().unwrap_or(Val::Undef));
                }
                Val::Bool(true)
            }
            "Reflect.has" => {
                let key = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                Val::Bool(matches!(&a0, Val::Object(o) if o.borrow().contains_key(&key)))
            }
            "Reflect.ownKeys" | "Reflect.keys" => Val::array(object_keys(&a0).into_iter().map(Val::str).collect()),
            "Reflect.deleteProperty" => {
                if let Val::Object(o) = &a0 {
                    let key = args.get(1).map(|v| v.to_str()).unwrap_or_default();
                    o.borrow_mut().remove(&key);
                }
                Val::Bool(true)
            }

            // ---- Web Audio sink: float samples [-1,1] -> i16 PCM -> virtio-sound ----
            "__webaudio_play" => {
                if let Val::Array(a) = &a0 {
                    let samples = a.borrow();
                    let mut pcm: Vec<u8> = Vec::with_capacity(samples.len() * 4);
                    for v in samples.iter() {
                        let s = (v.as_num().clamp(-1.0, 1.0) * 32767.0) as i16;
                        let b = s.to_le_bytes();
                        pcm.extend_from_slice(&b); // left
                        pcm.extend_from_slice(&b); // right (mono -> stereo)
                    }
                    crate::kprintln!("WEBAUDIO: rendered {} samples -> {} PCM bytes", samples.len(), pcm.len());
                    if crate::snd::available() && !pcm.is_empty() {
                        crate::snd::play(&pcm);
                    }
                }
                Val::Undef
            }
            // decodeAudioData backend — not yet wired to the MP3/WAV decoders.
            "__webaudio_decode" => Val::array(Vec::new()),

            // window-proxy postMessage: deliver a 'message' event to this
            // window's listeners (the opened window echoing back to its opener).
            "__win_post" => {
                let mut ev = Obj::new();
                ev.insert("type".into(), Val::str("message"));
                ev.insert("data".into(), a0);
                ev.insert("origin".into(), Val::str(""));
                let evv = Val::object(ev);
                let handlers: Vec<Val> = self
                    .listeners
                    .iter()
                    .filter(|l| l.node == self.dom.root && l.event == "message")
                    .map(|l| l.handler.clone())
                    .collect();
                for h in handlers {
                    self.deferred.push((h, alloc::vec![evv.clone()]));
                }
                Val::Undef
            }
            "queueMicrotask" => {
                if let Some(f) = args.first() {
                    self.deferred.push((f.clone(), Vec::new()));
                }
                Val::Undef
            }

            // ---- time (React's scheduler reads these for deadlines) ----
            // A constant clock is fine: the scheduler never decides to yield, so
            // the synchronous work loop runs to completion in one drain.
            "Date.now" | "performance.now" => Val::Num(0.0),

            // ---- Object.is / Symbol.for (used pervasively by React) ----
            "Object.is" => {
                let b = args.get(1).cloned().unwrap_or(Val::Undef);
                Val::Bool(match (&a0, &b) {
                    (Val::Num(x), Val::Num(y)) => {
                        if x.is_nan() && y.is_nan() {
                            true
                        } else if *x == 0.0 && *y == 0.0 {
                            // Object.is distinguishes +0 and -0 by sign bit.
                            x.is_sign_negative() == y.is_sign_negative()
                        } else {
                            x == y
                        }
                    }
                    _ => strict_eq(&a0, &b),
                })
            }
            // Symbol.for(key) returns a stable, ===-comparable token for a key.
            "Symbol.for" | "Symbol.iterator" | "Symbol.asyncIterator" => {
                Val::str(alloc::format!("@@{}", a0.to_str()))
            }

            // ---- Object statics ----
            "Object.keys" => Val::array(object_keys(&a0).into_iter().map(Val::str).collect()),
            "Object.values" => match &a0 {
                Val::Object(m) => Val::array(m.borrow().iter().filter(|(k, _)| !k.starts_with("__")).map(|(_, v)| v.clone()).collect()),
                Val::Array(a) => Val::array(a.borrow().clone()),
                _ => Val::array(Vec::new()),
            },
            "Object.entries" => match &a0 {
                Val::Object(m) => Val::array(
                    m.borrow().iter().filter(|(k, _)| !k.starts_with("__"))
                        .map(|(k, v)| Val::array(alloc::vec![Val::str(k.clone()), v.clone()])).collect(),
                ),
                _ => Val::array(Vec::new()),
            },
            "Object.assign" => {
                let target = a0.clone();
                if let Val::Object(t) = &target {
                    for src in args.iter().skip(1) {
                        if let Val::Object(s) = src {
                            for (k, v) in s.borrow().iter() {
                                t.borrow_mut().insert(k.clone(), v.clone());
                            }
                        }
                    }
                }
                target
            }
            "Object.freeze" | "Object.seal" | "Object.preventExtensions" => a0,
            "Object.create" => {
                // Link the new object to `proto` via the hidden __proto__ so
                // member lookups inherit dynamically (multi-level prototype
                // chains, e.g. Web Audio's OscillatorNode -> AudioNode).
                let mut o = Obj::new();
                if matches!(&a0, Val::Object(_)) {
                    o.insert("__proto__".into(), a0.clone());
                }
                Val::object(o)
            }
            "Object.getPrototypeOf" => Val::Null,
            "Object.defineProperty" => {
                if let (Val::Object(t), Some(key), Some(Val::Object(desc))) = (&a0, args.get(1), args.get(2)) {
                    let k = key.to_str();
                    if let Some(v) = desc.borrow().get("value") {
                        t.borrow_mut().insert(k, v.clone());
                    } else if let Some(g) = desc.borrow().get("get") {
                        t.borrow_mut().insert(alloc::format!("__get:{k}"), g.clone());
                    }
                }
                a0
            }
            "Object.fromEntries" => {
                let mut o = Obj::new();
                for pair in self.to_vec(&a0) {
                    let kv = self.to_vec(&pair);
                    if let Some(k) = kv.first() {
                        o.insert(k.to_str(), kv.get(1).cloned().unwrap_or(Val::Undef));
                    }
                }
                Val::object(o)
            }
            "Object.getOwnPropertyNames" => Val::array(object_keys(&a0).into_iter().map(Val::str).collect()),

            // ---- Array statics ----
            "Array.isArray" => Val::Bool(matches!(a0, Val::Array(_))),
            "Array.from" => {
                let items = self.to_vec(&a0);
                if let Some(mapper) = args.get(1).cloned() {
                    let mut out = Vec::with_capacity(items.len());
                    for (i, x) in items.into_iter().enumerate() {
                        out.push(self.call(mapper.clone(), Val::Undef, alloc::vec![x, Val::Num(i as f64)])?);
                    }
                    Val::array(out)
                } else if let Val::Object(m) = &a0 {
                    // Array.from({length: n}) or Map/Set
                    let b = m.borrow();
                    if let Some(Val::Array(e)) = b.get("__set") {
                        Val::array(e.borrow().clone())
                    } else if let Some(Val::Array(e)) = b.get("__map") {
                        Val::array(e.borrow().clone())
                    } else if let Some(len) = b.get("length") {
                        Val::array(vec![Val::Undef; len.as_num().max(0.0) as usize])
                    } else {
                        Val::array(items)
                    }
                } else {
                    Val::array(items)
                }
            }
            "Array.of" => Val::array(args),

            // ---- Number statics ----
            "Number.isInteger" => Val::Bool(matches!(a0, Val::Num(n) if n.is_finite() && n == mathf::trunc(n))),
            "Number.isFinite" => Val::Bool(matches!(a0, Val::Num(n) if n.is_finite())),
            "Number.isNaN" => Val::Bool(matches!(a0, Val::Num(n) if n.is_nan())),
            "Number.parseFloat" => return self.call_global("parseFloat", args),
            "Number.parseInt" => return self.call_global("parseInt", args),

            // ---- String statics ----
            "String.fromCharCode" => {
                let s: String = args.iter().filter_map(|v| char::from_u32(v.as_num() as u32)).collect();
                Val::str(s)
            }

            // ---- JSON ----
            "JSON.parse" => json_parse(&a0.to_str()),
            "JSON.stringify" => Val::str(json_stringify(&a0, args.get(2).map(|v| v.as_num() as usize).unwrap_or(0))),

            // ---- Promise statics ----
            "Promise.resolve" => self.make_promise("fulfilled", a0),
            "Promise.reject" => self.make_promise("rejected", a0),
            "Promise.all" => {
                let items = self.to_vec(&a0);
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    out.push(self.await_val(it)?);
                }
                self.make_promise("fulfilled", Val::array(out))
            }
            "Promise.allSettled" => {
                let items = self.to_vec(&a0);
                let mut out = Vec::new();
                for it in items {
                    let mut o = Obj::new();
                    match self.await_val(it) {
                        Ok(v) => {
                            o.insert("status".into(), Val::str("fulfilled"));
                            o.insert("value".into(), v);
                        }
                        Err(e) => {
                            o.insert("status".into(), Val::str("rejected"));
                            o.insert("reason".into(), e);
                        }
                    }
                    out.push(Val::object(o));
                }
                self.make_promise("fulfilled", Val::array(out))
            }
            "Promise.race" | "Promise.any" => {
                let items = self.to_vec(&a0);
                let v = items.into_iter().next().unwrap_or(Val::Undef);
                let r = self.await_val(v)?;
                self.make_promise("fulfilled", r)
            }

            // ---- fetch ----
            "fetch" => {
                let url = a0.to_str();
                self.do_fetch(&url, args.get(1).cloned())
            }

            // ---- WebSocket onopen/onerror dispatch (deferred) ----
            "__ws_open" => {
                if let Val::Object(o) = &a0 {
                    let onopen = o.borrow().get("onopen").cloned();
                    if let Some(h) = onopen {
                        self.call(h, a0.clone(), Vec::new())?;
                    }
                }
                Val::Undef
            }
            "__ws_error" => {
                if let Val::Object(o) = &a0 {
                    let onerr = o.borrow().get("onerror").cloned();
                    if let Some(h) = onerr {
                        let mut ev = Obj::new();
                        ev.insert("type".into(), Val::str("error"));
                        self.call(h, a0.clone(), alloc::vec![Val::object(ev)])?;
                    }
                }
                Val::Undef
            }

            // ---- Promise resolve/reject closures (id baked into the name) ----
            _ if name.starts_with("__resolve:") || name.starts_with("__reject:") => {
                let reject = name.starts_with("__reject:");
                if let Ok(id) = name[name.find(':').unwrap() + 1..].parse::<usize>() {
                    if let Some(cell) = self.resolvers.get(id).cloned() {
                        *cell.borrow_mut() = (
                            String::from(if reject { "rejected" } else { "fulfilled" }),
                            a0,
                        );
                    }
                }
                Val::Undef
            }

            _ => Val::Undef,
        })
    }

    /// Perform a synchronous fetch over the browser's HTTP stack and return a
    /// resolved Promise of a Response (`{ ok, status, __body }`).
    fn do_fetch(&mut self, url: &str, opts: Option<Val>) -> Val {
        let body = opts.as_ref().and_then(|o| {
            if let Val::Object(m) = o {
                m.borrow().get("body").map(|v| v.to_str())
            } else {
                None
            }
        });
        let resolved = crate::browser::js_fetch(url, body.as_deref());
        match resolved {
            Some((status, _ctype, data)) => {
                let mut o = Obj::new();
                o.insert("__body".into(), Val::str(String::from_utf8_lossy(&data).into_owned()));
                o.insert("ok".into(), Val::Bool((200..400).contains(&status)));
                o.insert("status".into(), Val::Num(status as f64));
                o.insert("statusText".into(), Val::str(if status == 200 { "OK" } else { "" }));
                o.insert("url".into(), Val::str(url.to_string()));
                self.make_promise("fulfilled", Val::object(o))
            }
            None => self.make_promise("rejected", Val::str(alloc::format!("fetch failed: {url}"))),
        }
    }
}

// ---- free helpers ----------------------------------------------------------

/// Own enumerable string keys of an object (skipping engine-internal __keys),
/// or array indices.
fn object_keys(v: &Val) -> Vec<String> {
    match v {
        Val::Object(m) => m.borrow().keys().filter(|k| !k.starts_with("__")).cloned().collect(),
        Val::Array(a) => (0..a.borrow().len()).map(|i| i.to_string()).collect(),
        _ => Vec::new(),
    }
}

fn deep_clone(v: &Val) -> Val {
    match v {
        Val::Array(a) => Val::array(a.borrow().iter().map(deep_clone).collect()),
        Val::Object(m) => {
            let mut o = Obj::new();
            for (k, val) in m.borrow().iter() {
                o.insert(k.clone(), deep_clone(val));
            }
            Val::object(o)
        }
        other => other.clone(),
    }
}

/// Minimal JSON parser → Val. Tolerant; returns Null on malformed input.
fn json_parse(s: &str) -> Val {
    let b = s.as_bytes();
    let mut i = 0;
    let v = json_value(b, &mut i);
    v.unwrap_or(Val::Null)
}

fn json_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i] == b' ' || b[*i] == b'\t' || b[*i] == b'\n' || b[*i] == b'\r') {
        *i += 1;
    }
}

fn json_value(b: &[u8], i: &mut usize) -> Option<Val> {
    json_ws(b, i);
    if *i >= b.len() {
        return None;
    }
    match b[*i] {
        b'{' => {
            *i += 1;
            let mut o = Obj::new();
            json_ws(b, i);
            if *i < b.len() && b[*i] == b'}' {
                *i += 1;
                return Some(Val::object(o));
            }
            loop {
                json_ws(b, i);
                let key = json_string(b, i)?;
                json_ws(b, i);
                if *i < b.len() && b[*i] == b':' {
                    *i += 1;
                }
                let val = json_value(b, i)?;
                o.insert(key, val);
                json_ws(b, i);
                if *i < b.len() && b[*i] == b',' {
                    *i += 1;
                    continue;
                }
                if *i < b.len() && b[*i] == b'}' {
                    *i += 1;
                }
                break;
            }
            Some(Val::object(o))
        }
        b'[' => {
            *i += 1;
            let mut arr = Vec::new();
            json_ws(b, i);
            if *i < b.len() && b[*i] == b']' {
                *i += 1;
                return Some(Val::array(arr));
            }
            loop {
                let val = json_value(b, i)?;
                arr.push(val);
                json_ws(b, i);
                if *i < b.len() && b[*i] == b',' {
                    *i += 1;
                    continue;
                }
                if *i < b.len() && b[*i] == b']' {
                    *i += 1;
                }
                break;
            }
            Some(Val::array(arr))
        }
        b'"' => json_string(b, i).map(Val::str),
        b't' => {
            *i += 4;
            Some(Val::Bool(true))
        }
        b'f' => {
            *i += 5;
            Some(Val::Bool(false))
        }
        b'n' => {
            *i += 4;
            Some(Val::Null)
        }
        _ => {
            let start = *i;
            while *i < b.len() && (b[*i].is_ascii_digit() || matches!(b[*i], b'-' | b'+' | b'.' | b'e' | b'E')) {
                *i += 1;
            }
            core::str::from_utf8(&b[start..*i]).ok().and_then(|s| s.parse::<f64>().ok()).map(Val::Num)
        }
    }
}

fn json_string(b: &[u8], i: &mut usize) -> Option<String> {
    if *i >= b.len() || b[*i] != b'"' {
        return None;
    }
    *i += 1;
    let mut s = String::new();
    while *i < b.len() && b[*i] != b'"' {
        if b[*i] == b'\\' && *i + 1 < b.len() {
            *i += 1;
            let c = match b[*i] {
                b'n' => '\n',
                b't' => '\t',
                b'r' => '\r',
                b'"' => '"',
                b'\\' => '\\',
                b'/' => '/',
                b'u' => {
                    // \uXXXX
                    if *i + 4 < b.len() {
                        let hex = core::str::from_utf8(&b[*i + 1..*i + 5]).unwrap_or("0");
                        let cp = u32::from_str_radix(hex, 16).unwrap_or(0);
                        *i += 4;
                        char::from_u32(cp).unwrap_or('?')
                    } else {
                        '?'
                    }
                }
                other => other as char,
            };
            s.push(c);
            *i += 1;
        } else {
            let ch_len = core::str::from_utf8(&b[*i..]).ok().and_then(|t| t.chars().next()).map(|c| c.len_utf8()).unwrap_or(1);
            if let Ok(t) = core::str::from_utf8(&b[*i..*i + ch_len]) {
                s.push_str(t);
            }
            *i += ch_len;
        }
    }
    *i += 1; // closing quote
    Some(s)
}

fn json_stringify(v: &Val, indent: usize) -> String {
    json_str_rec(v, indent, 0)
}

fn json_str_rec(v: &Val, indent: usize, depth: usize) -> String {
    let nl = if indent > 0 { "\n" } else { "" };
    let pad = |n: usize| if indent > 0 { " ".repeat(indent * n) } else { String::new() };
    match v {
        Val::Undef => String::from("null"),
        Val::Null => String::from("null"),
        Val::Bool(b) => String::from(if *b { "true" } else { "false" }),
        Val::Num(n) => {
            if n.is_finite() {
                num_to_str(*n)
            } else {
                String::from("null")
            }
        }
        Val::Str(s) => json_quote(s),
        Val::Array(a) => {
            let b = a.borrow();
            if b.is_empty() {
                return String::from("[]");
            }
            let items: Vec<String> = b.iter().map(|x| alloc::format!("{}{}", pad(depth + 1), json_str_rec(x, indent, depth + 1))).collect();
            alloc::format!("[{nl}{}{nl}{}]", items.join(&alloc::format!(",{nl}")), pad(depth))
        }
        Val::Object(m) => {
            let b = m.borrow();
            let entries: Vec<String> = b
                .iter()
                .filter(|(k, _)| !k.starts_with("__"))
                .map(|(k, val)| alloc::format!("{}{}:{}{}", pad(depth + 1), json_quote(k), if indent > 0 { " " } else { "" }, json_str_rec(val, indent, depth + 1)))
                .collect();
            if entries.is_empty() {
                return String::from("{}");
            }
            alloc::format!("{{{nl}{}{nl}{}}}", entries.join(&alloc::format!(",{nl}")), pad(depth))
        }
        _ => String::from("null"),
    }
}

fn json_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn rc_func(f: &Func) -> Rc<Func> {
    Rc::new(Func {
        name: f.name.clone(),
        params: f.params.clone(),
        body: f.body.clone(),
        expr_body: f.expr_body.clone(),
        arrow: f.arrow,
        is_async: f.is_async,
        is_generator: f.is_generator,
    })
}

fn type_of(v: &Val) -> &'static str {
    match v {
        Val::Undef => "undefined",
        Val::Null => "object",
        Val::Bool(_) => "boolean",
        Val::Num(_) => "number",
        Val::Str(_) => "string",
        Val::Func(..) | Val::Native(_) => "function",
        _ => "object",
    }
}

/// JS `ToInt32`: NaN/±Inf → 0; otherwise truncate toward zero and take the low
/// 32 bits as a signed integer (wrapping mod 2^32).
fn to_int32(x: f64) -> i32 {
    to_uint32(x) as i32
}

/// JS `ToUint32`: NaN/±Inf → 0; otherwise truncate toward zero and take the low
/// 32 bits as an unsigned integer (wrapping mod 2^32).
fn to_uint32(x: f64) -> u32 {
    if !x.is_finite() || x == 0.0 {
        return 0;
    }
    // Truncate toward zero, then reduce mod 2^32 into [0, 2^32). No std/libm:
    // mathf::trunc is frintz; the modulo is manual so negatives wrap correctly.
    const TWO32: f64 = 4294967296.0;
    let t = mathf::trunc(x);
    let mut m = t - mathf::floor(t / TWO32) * TWO32;
    if m < 0.0 {
        m += TWO32;
    }
    m as u32
}

fn binop(op: &str, l: Val, r: Val) -> Val {
    match op {
        "+" => {
            if matches!(l, Val::Str(_)) || matches!(r, Val::Str(_)) {
                let mut s = l.to_str();
                s.push_str(&r.to_str());
                Val::str(s)
            } else {
                Val::Num(l.as_num() + r.as_num())
            }
        }
        "-" => Val::Num(l.as_num() - r.as_num()),
        "*" => Val::Num(l.as_num() * r.as_num()),
        "/" => Val::Num(l.as_num() / r.as_num()),
        "%" => Val::Num(l.as_num() % r.as_num()),
        "**" => Val::Num(libm_pow(l.as_num(), r.as_num())),
        "==" => Val::Bool(loose_eq(&l, &r)),
        "!=" => Val::Bool(!loose_eq(&l, &r)),
        "===" => Val::Bool(strict_eq(&l, &r)),
        "!==" => Val::Bool(!strict_eq(&l, &r)),
        "<" => cmp(l, r, |o| o < 0),
        ">" => cmp(l, r, |o| o > 0),
        "<=" => cmp(l, r, |o| o <= 0),
        ">=" => cmp(l, r, |o| o >= 0),
        // Bitwise operators follow JS semantics: operands are coerced via
        // ToInt32/ToUint32 (truncate toward zero, then take the low 32 bits),
        // NOT a plain `as i64` cast — React's lane math (e.g. `lanes & -lanes`,
        // `1 << index` wrapping at bit 31) depends on real 32-bit overflow.
        "&" => Val::Num((to_int32(l.as_num()) & to_int32(r.as_num())) as f64),
        "|" => Val::Num((to_int32(l.as_num()) | to_int32(r.as_num())) as f64),
        "^" => Val::Num((to_int32(l.as_num()) ^ to_int32(r.as_num())) as f64),
        "<<" => Val::Num((to_int32(l.as_num()).wrapping_shl(to_uint32(r.as_num()) & 31)) as f64),
        ">>" => Val::Num((to_int32(l.as_num()) >> (to_uint32(r.as_num()) & 31)) as f64),
        ">>>" => Val::Num((to_uint32(l.as_num()) >> (to_uint32(r.as_num()) & 31)) as f64),
        "instanceof" => Val::Bool(false),
        "in" => match &r {
            Val::Object(o) => Val::Bool(o.borrow().contains_key(&l.to_str())),
            _ => Val::Bool(false),
        },
        _ => Val::Undef,
    }
}

fn cmp(l: Val, r: Val, f: impl Fn(i32) -> bool) -> Val {
    if let (Val::Str(a), Val::Str(b)) = (&l, &r) {
        let o = match a.as_str().cmp(b.as_str()) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        };
        return Val::Bool(f(o));
    }
    let (a, b) = (l.as_num(), r.as_num());
    if a.is_nan() || b.is_nan() {
        return Val::Bool(false);
    }
    let o = if a < b { -1 } else if a > b { 1 } else { 0 };
    Val::Bool(f(o))
}

fn loose_eq(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Undef | Val::Null, Val::Undef | Val::Null) => true,
        (Val::Num(x), Val::Num(y)) => x == y,
        (Val::Str(x), Val::Str(y)) => x == y,
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Node(x), Val::Node(y)) => x == y,
        (Val::Undef | Val::Null, _) | (_, Val::Undef | Val::Null) => false,
        _ => a.as_num() == b.as_num(),
    }
}

fn strict_eq(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Undef, Val::Undef) | (Val::Null, Val::Null) => true,
        (Val::Num(x), Val::Num(y)) => x == y,
        (Val::Str(x), Val::Str(y)) => x == y,
        (Val::Bool(x), Val::Bool(y)) => x == y,
        (Val::Node(x), Val::Node(y)) => x == y,
        // Singleton host objects compare by identity: window===window,
        // document===document, etc. (each Host kind is a single object).
        (Val::Host(x), Val::Host(y)) => core::mem::discriminant(x) == core::mem::discriminant(y) && host_same(x, y),
        // Same Rc allocation -> same object (arrays/objects/functions).
        (Val::Array(x), Val::Array(y)) => Rc::ptr_eq(x, y),
        (Val::Object(x), Val::Object(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// Two host values are the same object when they're the same kind and, for the
/// kinds that carry an owner index, the same index.
fn host_same(x: &Host, y: &Host) -> bool {
    match (x, y) {
        (Host::Style(a), Host::Style(b))
        | (Host::ClassList(a), Host::ClassList(b))
        | (Host::Dataset(a), Host::Dataset(b))
        | (Host::Canvas(a), Host::Canvas(b)) => a == b,
        _ => true, // Document/Window/Console/Math/Storage/History/Location singletons
    }
}

fn norm_idx(v: Option<&Val>, default: i64, len: i64) -> i64 {
    match v {
        Some(v) if !matches!(v, Val::Undef) => {
            let n = v.as_num() as i64;
            if n < 0 {
                (len + n).max(0)
            } else {
                n
            }
        }
        _ => default,
    }
}

/// PRNG state for crypto.getRandomValues/randomUUID (no real entropy source).
static RNG_STATE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0x2545_F491_4F6C_DD1D);

/// Parse a URL into a WHATWG-ish object (href/protocol/host/pathname/search/...).
fn parse_url(url: &str) -> Val {
    let mut o = Obj::new();
    o.insert("href".into(), Val::str(url));
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let protocol = if scheme.is_empty() { String::new() } else { alloc::format!("{scheme}:") };
    // split rest into authority + path
    let (authority, pathq) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let host = authority;
    let hostname = host.split(':').next().unwrap_or(host);
    let port = host.split_once(':').map(|(_, p)| p).unwrap_or("");
    // path / search / hash
    let (path_search, hash) = pathq.split_once('#').unwrap_or((pathq, ""));
    let (pathname, search) = path_search.split_once('?').map(|(p, q)| (p, alloc::format!("?{q}"))).unwrap_or((path_search, String::new()));
    let pathname = if pathname.is_empty() { "/" } else { pathname };
    o.insert("protocol".into(), Val::str(protocol.clone()));
    o.insert("host".into(), Val::str(host));
    o.insert("hostname".into(), Val::str(hostname));
    o.insert("port".into(), Val::str(port));
    o.insert("pathname".into(), Val::str(pathname));
    o.insert("search".into(), Val::str(search.clone()));
    o.insert("hash".into(), Val::str(if hash.is_empty() { String::new() } else { alloc::format!("#{hash}") }));
    o.insert("origin".into(), Val::str(if scheme.is_empty() { String::from("null") } else { alloc::format!("{protocol}//{host}") }));
    o.insert("searchParams".into(), make_url_search_params(search.trim_start_matches('?')));
    o.insert("toString".into(), Val::Native(Native::Global(Rc::from("noop")))); // href stored
    Val::object(o)
}

/// Build a URLSearchParams object from a query string (`a=1&b=2`).
fn make_url_search_params(query: &str) -> Val {
    let q = query.trim_start_matches('?');
    let mut pairs: Vec<Val> = Vec::new();
    for kv in q.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        pairs.push(Val::array(alloc::vec![Val::str(k), Val::str(v)]));
    }
    let mut o = Obj::new();
    o.insert("__usp".into(), Val::array(pairs));
    Val::object(o)
}

/// WebGL 1.0 enum constants (the subset our software GL accepts/uses).
fn webgl_const(name: &str) -> Option<f64> {
    Some(match name {
        "VERTEX_SHADER" => 35633.0, "FRAGMENT_SHADER" => 35632.0,
        "ARRAY_BUFFER" => 34962.0, "ELEMENT_ARRAY_BUFFER" => 34963.0,
        "STATIC_DRAW" => 35044.0, "DYNAMIC_DRAW" => 35048.0, "STREAM_DRAW" => 35040.0,
        "FLOAT" => 5126.0, "UNSIGNED_BYTE" => 5121.0, "UNSIGNED_SHORT" => 5123.0, "UNSIGNED_INT" => 5125.0, "INT" => 5124.0, "SHORT" => 5122.0, "BYTE" => 5120.0,
        "POINTS" => 0.0, "LINES" => 1.0, "LINE_LOOP" => 2.0, "LINE_STRIP" => 3.0,
        "TRIANGLES" => 4.0, "TRIANGLE_STRIP" => 5.0, "TRIANGLE_FAN" => 6.0,
        "DEPTH_BUFFER_BIT" => 256.0, "STENCIL_BUFFER_BIT" => 1024.0, "COLOR_BUFFER_BIT" => 16384.0,
        "DEPTH_TEST" => 2929.0, "BLEND" => 3042.0, "CULL_FACE" => 2884.0, "SCISSOR_TEST" => 3089.0, "DITHER" => 3024.0,
        "ZERO" => 0.0, "ONE" => 1.0, "SRC_ALPHA" => 770.0, "ONE_MINUS_SRC_ALPHA" => 771.0, "SRC_COLOR" => 768.0, "DST_ALPHA" => 772.0,
        "NEVER" => 512.0, "LESS" => 513.0, "EQUAL" => 514.0, "LEQUAL" => 515.0, "GREATER" => 516.0, "GEQUAL" => 518.0, "ALWAYS" => 519.0,
        "BACK" => 1029.0, "FRONT" => 1028.0, "CW" => 2304.0, "CCW" => 2305.0,
        "COMPILE_STATUS" => 35713.0, "LINK_STATUS" => 35714.0, "VALIDATE_STATUS" => 35715.0,
        "TEXTURE_2D" => 3553.0, "TEXTURE0" => 33984.0, "TEXTURE1" => 33985.0, "RGBA" => 6408.0, "RGB" => 6407.0,
        "NEAREST" => 9728.0, "LINEAR" => 9729.0, "CLAMP_TO_EDGE" => 33071.0, "REPEAT" => 10497.0,
        "TEXTURE_MAG_FILTER" => 10240.0, "TEXTURE_MIN_FILTER" => 10241.0, "TEXTURE_WRAP_S" => 10242.0, "TEXTURE_WRAP_T" => 10243.0,
        "FRAMEBUFFER" => 36160.0, "RENDERBUFFER" => 36161.0, "COLOR_ATTACHMENT0" => 36064.0, "DEPTH_ATTACHMENT" => 36096.0,
        "ARRAY_BUFFER_BINDING" => 34964.0, "VERSION" => 7938.0, "MAX_TEXTURE_SIZE" => 3379.0,
        _ => return None,
    })
}

fn math_method(name: &str, args: &[Val]) -> Val {
    let a0 = args.first().map(|v| v.as_num()).unwrap_or(f64::NAN);
    let a1 = args.get(1).map(|v| v.as_num()).unwrap_or(f64::NAN);
    Val::Num(match name {
        "floor" => mathf::floor(a0),
        "ceil" => mathf::ceil(a0),
        "round" => mathf::floor(a0 + 0.5),
        "trunc" => mathf::trunc(a0),
        "abs" => a0.abs(),
        "sqrt" => mathf::sqrt(a0),
        "min" => args.iter().map(|v| v.as_num()).fold(f64::INFINITY, f64::min),
        "max" => args.iter().map(|v| v.as_num()).fold(f64::NEG_INFINITY, f64::max),
        "pow" => libm_pow(a0, a1),
        "random" => 0.5,
        "sign" => {
            if a0 > 0.0 {
                1.0
            } else if a0 < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
        "hypot" => mathf::sqrt(a0 * a0 + a1 * a1),
        // clz32: count leading zero bits in the 32-bit value. React's lane
        // priority math is `31 - clz32(lanes)`; without it the reconciler loops.
        "clz32" => (a0 as i64 as u32).leading_zeros() as f64,
        "max_safe" => 9007199254740991.0,
        // Trigonometry (no_std, from src/js/mathf.rs) — used by Web Audio
        // oscillators, Canvas transforms, and general page scripts.
        "sin" => mathf::sin(a0),
        "cos" => mathf::cos(a0),
        "tan" => mathf::sin(a0) / mathf::cos(a0),
        "atan2" => atan2(a0, a1),
        "atan" => atan2(a0, 1.0),
        "asin" => atan2(a0, mathf::sqrt((1.0 - a0 * a0).max(0.0))),
        "acos" => core::f64::consts::FRAC_PI_2 - atan2(a0, mathf::sqrt((1.0 - a0 * a0).max(0.0))),
        "exp" => libm_pow(core::f64::consts::E, a0),
        "log" => ln(a0),
        "log2" => ln(a0) / core::f64::consts::LN_2,
        "log10" => ln(a0) / core::f64::consts::LN_10,
        "cbrt" => libm_pow(a0, 1.0 / 3.0),
        _ => f64::NAN,
    })
}

/// natural log via ln(m·2^e) = e·ln2 + ln(m), m in [1,2) by a short series.
fn ln(x: f64) -> f64 {
    if x <= 0.0 {
        return if x == 0.0 { f64::NEG_INFINITY } else { f64::NAN };
    }
    // Decompose x = m * 2^e with m in [1, 2).
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    // ln(m) via atanh series: ln(m) = 2·(t + t³/3 + t⁵/5 + …), t=(m-1)/(m+1).
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    let series = t * (1.0 + t2 * (1.0 / 3.0 + t2 * (1.0 / 5.0 + t2 * (1.0 / 7.0 + t2 / 9.0))));
    (e as f64) * core::f64::consts::LN_2 + 2.0 * series
}

/// atan2 via an atan polynomial with argument reduction + quadrant handling.
fn atan2(y: f64, x: f64) -> f64 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let ax = x.abs();
    let ay = y.abs();
    // atan(z) for z in [0,1] via a minimax-ish odd polynomial; reduce z>1 to 1/z.
    let (z, swap) = if ay > ax { (ax / ay, true) } else { (ay / ax, false) };
    let z2 = z * z;
    let mut a = z * (0.9998660 + z2 * (-0.3302995 + z2 * (0.1801410 + z2 * (-0.0851330 + z2 * 0.0208351))));
    if swap {
        a = core::f64::consts::FRAC_PI_2 - a;
    }
    // place into the correct quadrant
    let r = if x < 0.0 { core::f64::consts::PI - a } else { a };
    if y < 0.0 { -r } else { r }
}

fn num_method(n: f64, name: &str, args: &[Val]) -> Val {
    match name {
        "toFixed" => {
            let d = args.first().map(|v| v.as_num() as usize).unwrap_or(0);
            let factor = libm_pow(10.0, d as f64);
            let r = mathf::floor(n * factor + 0.5) / factor;
            // format with d decimals
            if d == 0 {
                Val::str(num_to_str(r))
            } else {
                let mut s = alloc::format!("{:.*}", d, r);
                if !s.contains('.') {
                    s.push('.');
                    for _ in 0..d {
                        s.push('0');
                    }
                }
                Val::str(s)
            }
        }
        "toString" => Val::str(num_to_str(n)),
        _ => Val::Undef,
    }
}

fn str_method(s: &str, name: &str, args: &[Val]) -> Val {
    let a0 = args.first().map(|v| v.to_str()).unwrap_or_default();
    match name {
        "toUpperCase" => Val::str(s.to_uppercase()),
        "toLowerCase" => Val::str(s.to_lowercase()),
        "trim" => Val::str(s.trim().to_string()),
        "trimStart" => Val::str(s.trim_start().to_string()),
        "trimEnd" => Val::str(s.trim_end().to_string()),
        "split" => {
            if args.is_empty() {
                Val::array(vec![Val::str(s.to_string())])
            } else if a0.is_empty() {
                Val::array(s.chars().map(|c| Val::str(c.to_string())).collect())
            } else {
                Val::array(s.split(&a0).map(|p| Val::str(p.to_string())).collect())
            }
        }
        "slice" => {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len() as i64;
            let start = norm_idx(args.first(), 0, len).clamp(0, len);
            let end = norm_idx(args.get(1), len, len).clamp(start, len);
            Val::str(chars[start as usize..end as usize].iter().collect::<String>())
        }
        "substring" => {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len() as i64;
            let mut a = args.first().map(|v| v.as_num() as i64).unwrap_or(0).clamp(0, len);
            let mut b = args.get(1).map(|v| v.as_num() as i64).unwrap_or(len).clamp(0, len);
            if a > b {
                core::mem::swap(&mut a, &mut b);
            }
            Val::str(chars[a as usize..b as usize].iter().collect::<String>())
        }
        "substr" => {
            let chars: Vec<char> = s.chars().collect();
            let len = chars.len() as i64;
            let start = norm_idx(args.first(), 0, len).clamp(0, len);
            let cnt = args.get(1).map(|v| v.as_num() as i64).unwrap_or(len - start).max(0);
            let end = (start + cnt).clamp(start, len);
            Val::str(chars[start as usize..end as usize].iter().collect::<String>())
        }
        "charAt" => {
            let i = args.first().map(|v| v.as_num() as usize).unwrap_or(0);
            Val::str(s.chars().nth(i).map(|c| c.to_string()).unwrap_or_default())
        }
        "charCodeAt" | "codePointAt" => {
            let i = args.first().map(|v| v.as_num() as usize).unwrap_or(0);
            s.chars().nth(i).map(|c| Val::Num(c as u32 as f64)).unwrap_or(Val::Num(f64::NAN))
        }
        "indexOf" => {
            Val::Num(s.find(&a0).map(|p| s[..p].chars().count() as f64).unwrap_or(-1.0))
        }
        "lastIndexOf" => {
            Val::Num(s.rfind(&a0).map(|p| s[..p].chars().count() as f64).unwrap_or(-1.0))
        }
        "includes" => Val::Bool(s.contains(&a0)),
        "startsWith" => Val::Bool(s.starts_with(&a0)),
        "endsWith" => Val::Bool(s.ends_with(&a0)),
        "replace" => Val::str(s.replacen(&a0, &args.get(1).map(|v| v.to_str()).unwrap_or_default(), 1)),
        "replaceAll" => Val::str(s.replace(&a0, &args.get(1).map(|v| v.to_str()).unwrap_or_default())),
        "repeat" => Val::str(s.repeat(args.first().map(|v| v.as_num() as usize).unwrap_or(0))),
        "padStart" => {
            let target = args.first().map(|v| v.as_num() as usize).unwrap_or(0);
            let pad = if args.len() > 1 { args[1].to_str() } else { String::from(" ") };
            let mut out = String::new();
            while out.chars().count() + s.chars().count() < target && !pad.is_empty() {
                out.push_str(&pad);
            }
            let need = target.saturating_sub(s.chars().count());
            let prefix: String = out.chars().take(need).collect();
            Val::str(alloc::format!("{prefix}{s}"))
        }
        "padEnd" => {
            let target = args.first().map(|v| v.as_num() as usize).unwrap_or(0);
            let pad = if args.len() > 1 { args[1].to_str() } else { String::from(" ") };
            let mut out = String::from(s);
            while out.chars().count() < target && !pad.is_empty() {
                out.push_str(&pad);
            }
            Val::str(out.chars().take(target.max(s.chars().count())).collect::<String>())
        }
        "concat" => {
            let mut out = String::from(s);
            for a in args {
                out.push_str(&a.to_str());
            }
            Val::str(out)
        }
        "at" => {
            let chars: Vec<char> = s.chars().collect();
            let i = args.first().map(|v| v.as_num() as i64).unwrap_or(0);
            let idx = if i < 0 { chars.len() as i64 + i } else { i };
            if idx >= 0 && (idx as usize) < chars.len() {
                Val::str(chars[idx as usize].to_string())
            } else {
                Val::Undef
            }
        }
        "toString" => Val::str(s.to_string()),
        "match" | "matchAll" | "search" => Val::Null,
        "normalize" => Val::str(s.to_string()),
        _ => Val::Undef,
    }
}

/// Integer-aware pow (the kernel has no libm); handles the small exponents the
/// scripts use (toFixed, Math.pow with small ints) plus a float fallback.
fn libm_pow(base: f64, exp: f64) -> f64 {
    if exp == mathf::trunc(exp) && exp.abs() < 64.0 {
        let mut r = 1.0;
        let n = exp.abs() as i64;
        for _ in 0..n {
            r *= base;
        }
        return if exp < 0.0 { 1.0 / r } else { r };
    }
    // fractional fallback: exp2(exp*log2(base)) — rarely used here
    if base <= 0.0 {
        return f64::NAN;
    }
    exp2(exp * log2(base))
}

fn log2(x: f64) -> f64 {
    // ln(x)/ln(2) via a simple series after range reduction
    let mut e = 0i32;
    let mut m = x;
    while m >= 2.0 {
        m /= 2.0;
        e += 1;
    }
    while m < 1.0 {
        m *= 2.0;
        e -= 1;
    }
    // m in [1,2); ln(m) via atanh series
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    let mut sum = 0.0;
    let mut term = t;
    let mut k = 1.0;
    for _ in 0..12 {
        sum += term / k;
        term *= t2;
        k += 2.0;
    }
    e as f64 + (2.0 * sum) / core::f64::consts::LN_2
}

fn exp2(x: f64) -> f64 {
    let i = mathf::floor(x);
    let f = x - i;
    // 2^f via polynomial
    let ln2 = core::f64::consts::LN_2;
    let y = f * ln2;
    let frac = 1.0 + y + y * y / 2.0 + y * y * y / 6.0 + y * y * y * y / 24.0;
    let mut p = 1.0;
    let n = i as i64;
    if n >= 0 {
        for _ in 0..n {
            p *= 2.0;
        }
    } else {
        for _ in 0..(-n) {
            p /= 2.0;
        }
    }
    p * frac
}

fn kebab(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
