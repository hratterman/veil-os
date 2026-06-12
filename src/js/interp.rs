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
}

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
        b.vars.insert("sessionStorage".into(), Val::Host(Host::LocalStorage));
        b.vars.insert("history".into(), Val::Host(Host::History));
        b.vars.insert("location".into(), Val::Host(Host::Location));
        for f in ["setTimeout", "setInterval", "requestAnimationFrame", "clearTimeout",
                  "clearInterval", "cancelAnimationFrame", "parseInt", "parseFloat", "isNaN",
                  "isFinite", "String", "Number", "Boolean", "Array", "Object", "JSON",
                  "encodeURIComponent", "decodeURIComponent", "alert", "fetch", "addEventListener"] {
            b.vars.insert(f.into(), Val::Native(Native::Global(Rc::from(f))));
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
        while !self.deferred.is_empty() && rounds < 50 {
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
                    self.bind_pat(pat, v, scope);
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
                    self.bind_pat(pat, it, &inner);
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
                    self.bind_pat(pat, k, &inner);
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

    fn bind_pat(&mut self, pat: &Pat, val: Val, scope: &Scope) {
        match pat {
            Pat::Ident(n) => {
                scope.borrow_mut().vars.insert(n.clone(), val);
            }
            Pat::Array(items, rest) => {
                let arr = self.to_vec(&val);
                for (i, p) in items.iter().enumerate() {
                    self.bind_pat(p, arr.get(i).cloned().unwrap_or(Val::Undef), scope);
                }
                if let Some(r) = rest {
                    let tail: Vec<Val> = arr.into_iter().skip(items.len()).collect();
                    scope.borrow_mut().vars.insert(r.clone(), Val::array(tail));
                }
            }
        }
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
                    let key = match k {
                        PropKey::Ident(s) => s.clone(),
                        PropKey::Computed(e) => self.eval(e, scope)?.to_str(),
                    };
                    let val = self.eval(v, scope)?;
                    o.insert(key, val);
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
                    "~" => Val::Num(!(v.as_num() as i64) as f64),
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
                // Minimal: most constructors used (Date, Error) return an object.
                let _ = (callee, args);
                let mut o = Obj::new();
                o.insert("__new".into(), Val::Bool(true));
                Ok(Val::object(o))
            }
        }
    }

    fn eval_call(&mut self, callee: &Expr, args: &[Expr], opt: bool, scope: &Scope) -> Result<Val, Val> {
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

    pub fn call(&mut self, func: Val, this: Val, args: Vec<Val>) -> Result<Val, Val> {
        match func {
            Val::Func(f, captured) => {
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
                            self.bind_pat(p, v, &inner);
                        }
                    }
                }
                if let Some(eb) = &f.expr_body {
                    self.eval(eb, &inner)
                } else {
                    match self.exec_block(&f.body, &inner)? {
                        Flow::Return(v) => Ok(v),
                        _ => Ok(Val::Undef),
                    }
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

    fn get_member(&mut self, o: Val, prop: &str) -> Result<Val, Val> {
        match &o {
            Val::Object(map) => {
                if let Some(v) = map.borrow().get(prop) {
                    return Ok(v.clone());
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
            _ => Ok(Val::Undef),
        }
    }

    fn set_member(&mut self, o: Val, prop: &str, v: Val) {
        match o {
            Val::Object(map) => {
                map.borrow_mut().insert(prop.into(), v);
            }
            Val::Node(idx) => self.set_node_member(idx, prop, v),
            Val::Host(Host::Style(idx)) => {
                self.dom.set_style(idx, prop, &v.to_str());
            }
            Val::Host(Host::Location) => { /* navigation ignored */ }
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
            "nodeName" => Val::str(n.tag.to_ascii_uppercase()),
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
                _ => Val::Native(Native::Method(Box::new(Val::Host(Host::Window)), Rc::from(prop))),
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
            Host::ClassList(_) | Host::Console | Host::LocalStorage | Host::History
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
            Val::Host(Host::Document) => Ok(Some(self.document_method(name, args)?)),
            Val::Host(Host::Window) => Ok(Some(self.window_method(name, args)?)),
            Val::Host(Host::Console) => {
                let parts: Vec<String> = args.iter().map(|v| v.to_str()).collect();
                self.errors.push(alloc::format!("[console] {}", parts.join(" ")));
                Ok(Some(Val::Undef))
            }
            Val::Host(Host::Math) => Ok(Some(math_method(name, args))),
            Val::Host(Host::LocalStorage) => match name {
                "getItem" => Ok(Some(
                    self.storage.get(&a0().to_str()).map(|s| Val::str(s.clone())).unwrap_or(Val::Null),
                )),
                "setItem" => {
                    self.storage.insert(a0().to_str(), args.get(1).map(|v| v.to_str()).unwrap_or_default());
                    Ok(Some(Val::Undef))
                }
                "removeItem" => {
                    self.storage.remove(&a0().to_str());
                    Ok(Some(Val::Undef))
                }
                _ => Ok(Some(Val::Undef)),
            },
            Val::Host(Host::History) => Ok(Some(Val::Undef)),
            Val::Host(Host::Location) => Ok(Some(Val::Undef)),
            Val::Host(Host::ClassList(idx)) => Ok(Some(self.classlist_method(*idx, name, args))),
            Val::Host(Host::Style(_)) => Ok(Some(Val::Undef)),
            Val::Node(idx) => Ok(self.node_method(*idx, name, args)?),
            Val::Array(a) => Ok(self.array_method(a.clone(), name, args)?),
            Val::Str(s) => Ok(Some(str_method(s, name, args))),
            Val::Num(n) => Ok(Some(num_method(*n, name, args))),
            _ => Ok(None),
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
            "createElement" => Ok(Val::Node(self.dom.create_element(&a0))),
            "createTextNode" => Ok(Val::Node(self.dom.create_text(&a0))),
            "addEventListener" => {
                if let Some(h) = args.get(1) {
                    self.listeners.push(Listener { node: self.dom.root, event: a0, handler: h.clone() });
                }
                Ok(Val::Undef)
            }
            "removeEventListener" | "write" | "dispatchEvent" => Ok(Val::Undef),
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
            "scrollTo" | "scroll" | "scrollBy" | "removeEventListener" | "alert" | "focus" | "open" => {
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
                    self.dom.append_child(idx, c);
                }
                Val::Undef
            }
            "removeChild" => {
                if let Val::Node(c) = a0 {
                    self.dom.nodes[idx].children.retain(|&x| x != c);
                }
                Val::Undef
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
            _ => return Ok(None),
        };
        Ok(Some(r))
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
            "encodeURIComponent" | "decodeURIComponent" => Val::str(a0.to_str()),
            "Array" => {
                // Array.from / Array(n) — treat single number as length
                if let Val::Num(n) = a0 {
                    Val::array(vec![Val::Undef; n.max(0.0) as usize])
                } else {
                    Val::array(self.to_vec(&a0))
                }
            }
            "Object" => a0,
            "fetch" => Val::Undef,
            _ => Val::Undef,
        })
    }
}

// ---- free helpers ----------------------------------------------------------

fn rc_func(f: &Func) -> Rc<Func> {
    Rc::new(Func {
        name: f.name.clone(),
        params: f.params.clone(),
        body: f.body.clone(),
        expr_body: f.expr_body.clone(),
        arrow: f.arrow,
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
        "&" => Val::Num(((l.as_num() as i64) & (r.as_num() as i64)) as f64),
        "|" => Val::Num(((l.as_num() as i64) | (r.as_num() as i64)) as f64),
        "^" => Val::Num(((l.as_num() as i64) ^ (r.as_num() as i64)) as f64),
        "<<" => Val::Num((((l.as_num() as i64) << (r.as_num() as i64 & 31)) as i32) as f64),
        ">>" => Val::Num((((l.as_num() as i32) >> (r.as_num() as i64 & 31)) as i32) as f64),
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
        _ => false,
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
        _ => f64::NAN,
    })
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
